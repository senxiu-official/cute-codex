use super::session::Session;
use super::turn_context::TurnContext;
use crate::config::Config;
use codex_protocol::config_types::AutoCompactTokenLimitScope;
use codex_protocol::openai_models::ModelInfo;

#[derive(Debug)]
pub(crate) struct ContextWindowTokenStatus {
    // Full active context usage, independent of the configured auto-compact scope.
    pub(crate) active_context_tokens: i64,
    // Usage counted against `model_auto_compact_token_limit` for the current scope.
    pub(crate) auto_compact_scope_tokens: i64,
    pub(crate) auto_compact_scope_limit: Option<i64>,
    pub(crate) full_context_window_limit: Option<i64>,
    pub(crate) base_window_tokens_remaining: Option<i64>,
    pub(crate) auto_compact_window_prefill_tokens: Option<i64>,
    pub(crate) full_context_window_limit_reached: bool,
    pub(crate) token_limit_reached: bool,
    pub(crate) turn_end_compaction_threshold_reached: bool,
}

fn tokens_remaining(limit: Option<i64>, used: i64) -> Option<i64> {
    limit.map(|limit| limit.saturating_sub(used).max(0))
}

pub(crate) async fn context_window_token_status(
    sess: &Session,
    turn_context: &TurnContext,
) -> ContextWindowTokenStatus {
    context_window_token_status_with_config(
        sess,
        turn_context.config.as_ref(),
        turn_context.model_info().as_ref(),
    )
    .await
}

pub(crate) async fn context_window_token_status_for_model(
    sess: &Session,
    config: &Config,
    turn_context: &TurnContext,
    model_info: &ModelInfo,
) -> ContextWindowTokenStatus {
    let mut config = config.clone();
    config.token_budget = super::token_budget::resolve_token_budget(
        turn_context.configured_token_budget.as_ref(),
        turn_context.use_model_token_budget_defaults,
        model_info,
    );
    context_window_token_status_with_config(sess, &config, model_info).await
}

async fn context_window_token_status_with_config(
    sess: &Session,
    config: &Config,
    model_info: &ModelInfo,
) -> ContextWindowTokenStatus {
    let active_context_tokens = sess.get_total_token_usage().await;

    // Count either the full active context or only the tokens added after the initial prefix.
    let (auto_compact_scope_tokens, auto_compact_scope_limit, auto_compact_window_prefill_tokens) =
        match config.model_auto_compact_token_limit_scope {
            AutoCompactTokenLimitScope::Total => (
                active_context_tokens,
                model_info.auto_compact_token_limit(),
                None,
            ),
            AutoCompactTokenLimitScope::BodyAfterPrefix => {
                let window = sess.auto_compact_window_snapshot().await;
                let baseline = window.prefill_input_tokens.unwrap_or(active_context_tokens);

                let scope_limit = prefix_auto_compact_limit(config, model_info);
                (
                    active_context_tokens.saturating_sub(baseline),
                    scope_limit,
                    window.prefill_input_tokens,
                )
            }
        };

    // The outer compaction budget also applies when only a runtime fallback is known.
    let full_context_window_limit = model_info.budget_context_window().map(|context_window| {
        context_window.saturating_mul(model_info.effective_context_window_percent) / 100
    });

    // Report remaining tokens against the base (unbuffered) window, capped by the full context.
    let base_window_tokens_remaining = [
        tokens_remaining(auto_compact_scope_limit, auto_compact_scope_tokens),
        tokens_remaining(full_context_window_limit, active_context_tokens),
    ]
    .into_iter()
    .flatten()
    .min();

    // Only reserve the fallback buffer when there is a fallback prompt to use it.
    let auto_compact_fallback_buffer_tokens = config
        .token_budget
        .as_ref()
        .map_or(0, crate::config::TokenBudgetConfig::fallback_buffer_tokens);
    let buffered_auto_compact_limit = auto_compact_scope_limit
        .map(|limit| limit.saturating_add(auto_compact_fallback_buffer_tokens));

    // Force compaction once the buffered window or the model's full context window is reached.
    let full_context_window_limit_reached =
        full_context_window_limit.is_some_and(|limit| active_context_tokens >= limit);
    let token_limit_reached = buffered_auto_compact_limit
        .is_some_and(|limit| auto_compact_scope_tokens >= limit)
        || full_context_window_limit_reached;
    let post_turn_percent = config.model_post_turn_compact_threshold_percent;
    let turn_end_compaction_threshold_reached = post_turn_percent > 0
        && (token_limit_reached
            || full_context_window_limit.is_some_and(|limit| {
                i128::from(active_context_tokens) * 100
                    >= i128::from(limit) * i128::from(post_turn_percent)
            }));

    ContextWindowTokenStatus {
        active_context_tokens,
        auto_compact_scope_tokens,
        auto_compact_scope_limit,
        full_context_window_limit,
        base_window_tokens_remaining,
        auto_compact_window_prefill_tokens,
        full_context_window_limit_reached,
        token_limit_reached,
        turn_end_compaction_threshold_reached,
    }
}

fn prefix_auto_compact_limit(config: &Config, model_info: &ModelInfo) -> Option<i64> {
    if config.model_context_settings_v1 {
        model_info.auto_compact_token_limit()
    } else {
        config
            .model_auto_compact_token_limit
            .or_else(|| model_info.auto_compact_token_limit())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn model_context_v1_prefix_scope_does_not_reapply_profile_absolute_limit() {
        let mut config = crate::config::test_config().await;
        config.model_auto_compact_token_limit = Some(100);
        let mut row = codex_models_manager::model_info::model_info_from_slug("test");
        row.context_window = Some(1000);
        row.auto_compact_percent = Some(75);
        row.context_settings = Some(codex_protocol::openai_models::ModelContextSettings {
            owner: Some(codex_protocol::openai_models::ModelContextSettingsValues {
                context_window: Some(1000),
                auto_compact_percent: Some(75),
                ..Default::default()
            }),
            ..Default::default()
        });
        for (v1, expected) in [(false, 100), (true, 750)] {
            config.model_context_settings_v1 = v1;
            let model = codex_models_manager::model_info::with_config_overrides(
                row.clone(),
                &config.to_models_manager_config(),
            );
            assert_eq!(prefix_auto_compact_limit(&config, &model), Some(expected));
        }
    }
}
