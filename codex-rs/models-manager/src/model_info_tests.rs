use super::*;
use crate::ModelsManagerConfig;
use codex_prompts::render_model_instructions;
use codex_protocol::config_types::Personality;
use codex_protocol::openai_models::ApprovalMessages;
use codex_protocol::openai_models::AppsMessages;
use codex_protocol::openai_models::AutoReviewMessages;
use codex_protocol::openai_models::CollaborationModeMessages;
use codex_protocol::openai_models::ConfirmationPolicies;
use codex_protocol::openai_models::GranularApprovalMessages;
use codex_protocol::openai_models::GuardianV2ModelConfig;
use codex_protocol::openai_models::ModelTokenBudgetConfig;
use codex_protocol::openai_models::MultiAgentMessages;
use codex_protocol::openai_models::MultiAgentModeMessages;
use codex_protocol::openai_models::MultiAgentRoleMessages;
use codex_protocol::openai_models::MultiAgentToolMessages;
use codex_protocol::openai_models::PermissionMessages;
use codex_protocol::openai_models::ToolMessage;
use codex_protocol::openai_models::ToolMessages;
use pretty_assertions::assert_eq;

fn config_with_personality(personality: Option<Personality>) -> ModelsManagerConfig {
    ModelsManagerConfig {
        personality,
        ..Default::default()
    }
}

#[test]
fn base_instruction_override_is_literal_and_preserves_catalog_messages() {
    let override_instructions = "override {{ personality }}\n# Personality\nKeep me";
    let persistent_instructions = "Follow up on the active task.";
    let async_message_description = "Catalog async message description.";
    let mut model = model_info_from_slug("unknown-model");
    let approvals = ApprovalMessages {
        on_request: Some("user approvals".to_string()),
        on_request_auto_review: Some("auto approvals".to_string()),
        never: Some("never approvals".to_string()),
        unless_trusted: Some("unless-trusted approvals".to_string()),
        granular: Some(GranularApprovalMessages {
            intro: Some("Catalog: {{ policy }}".to_string()),
            prompted_categories: Some("Allowed: {{ categories }}".to_string()),
            rejected_categories: Some("Denied: {{ categories }}".to_string()),
        }),
        ..Default::default()
    };
    let collaboration_modes = CollaborationModeMessages {
        default: Some("default instructions".to_string()),
        plan: Some("plan instructions".to_string()),
    };
    let auto_review = AutoReviewMessages {
        policy: Some("review policy".to_string()),
        policy_template: Some("review policy template".to_string()),
        node_repl_policy: None,
        rejection_instructions: Some("rejection instructions".to_string()),
        timeout_instructions: Some(String::new()),
    };
    let permissions = PermissionMessages {
        danger_full_access: Some("danger".to_string()),
        workspace_write: Some(String::new()),
        read_only: None,
        approved_prefixes: None,
        ..PermissionMessages::default()
    };
    let multi_agent = MultiAgentMessages {
        role: Some(MultiAgentRoleMessages {
            root: Some("root base".to_string()),
            subagent: Some("subagent base".to_string()),
        }),
        mode: Some(MultiAgentModeMessages {
            explicit: Some("explicit mode".to_string()),
            proactive: Some("proactive mode".to_string()),
            hint_text: Some("mode hint".to_string()),
        }),
    };
    let token_budget = ModelTokenBudgetConfig {
        enabled: false,
        use_history_notes_extension: false,
        reminder_threshold_tokens: 128,
        reminder_message_template: "budget reminder".to_string(),
        guidance_message: "budget guidance".to_string(),
        auto_compact_fallback_prompt: "compact prompt".to_string(),
        auto_compact_fallback_buffer_tokens: 64,
    };
    let guardian_v2 = GuardianV2ModelConfig {
        classifier_instructions: Some("Guardian experiment".to_string()),
        ..Default::default()
    };
    let confirmation_policies = ConfirmationPolicies {
        browser_use: Some("# Browser policy\n\n{{literal_markdown}}\n".to_string()),
        computer_use: Some("  # Native policy\r\n\n${native_markdown}\n".to_string()),
    };
    let mut messages = ModelMessages {
        apps: Some(AppsMessages {
            usage_instructions: Some("Catalog Apps instructions.".to_string()),
        }),
        plugins: None,
        content_filter_guidance: None,
        persistent_instructions: Some(persistent_instructions.to_string()),
        tools: Some(ToolMessages {
            send_user_message_async: Some(ToolMessage {
                description: Some(async_message_description.to_string()),
                ..Default::default()
            }),
            multi_agent: Some(MultiAgentToolMessages {
                spawn_agent: Some(ToolMessage {
                    description: Some("Catalog spawn description.".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        instructions_template: Some("template".to_string()),
        approvals: Some(approvals),
        collaboration_modes: Some(collaboration_modes),
        auto_review: Some(auto_review),
        permissions: Some(permissions),
        multi_agent: Some(multi_agent),
        token_budget: Some(token_budget),
        confirmation_policies: Some(confirmation_policies),
        guardian_v2: Some(guardian_v2),
    };
    model.model_messages = Some(messages.clone());
    let config = ModelsManagerConfig {
        base_instructions: Some(override_instructions.to_string()),
        personality: Some(Personality::None),
        ..Default::default()
    };

    let updated = with_config_overrides(model, &config);

    messages.instructions_template = Some(override_instructions.to_string());
    assert_eq!(updated.model_messages, Some(messages));
    assert_eq!(render_model_instructions(&updated), override_instructions);
}

#[test]
fn personality_none_strips_catalog_instruction_sources_through_the_next_h1() {
    let config = config_with_personality(Some(Personality::None));
    for (instructions, expected) in [
        (
            "Intro\n\n# Personality\n\nRemove me\n\n## Writing Style\n\nRemove me too\n\n# Safety\n\nKeep me",
            "Intro\n\n# Safety\n\nKeep me",
        ),
        ("Intro\n\n# Personality\n\nRemove me", "Intro\n\n"),
        (
            "Intro\n\n## Personality\n\nKeep me",
            "Intro\n\n## Personality\n\nKeep me",
        ),
        (
            "Intro\n\n# Personality \n\nKeep me",
            "Intro\n\n# Personality \n\nKeep me",
        ),
        (
            "Intro\r\n\r\n# Personality\r\n\r\nRemove me\r\n\r\n## Writing Style\r\n\r\nRemove me too\r\n\r\n# General\r\n\r\nKeep me",
            "Intro\r\n\r\n# General\r\n\r\nKeep me",
        ),
    ] {
        let mut messages = ModelMessages {
            instructions_template: Some(instructions.to_string()),
            persistent_instructions: Some(String::new()),
            tools: Some(ToolMessages {
                send_user_message_async: Some(ToolMessage {
                    description: Some(String::new()),
                    ..Default::default()
                }),
                multi_agent: Some(MultiAgentToolMessages {
                    spawn_agent: Some(ToolMessage {
                        description: Some(String::new()),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            approvals: Some(ApprovalMessages {
                on_request: Some("user approvals".to_string()),
                on_request_auto_review: None,
                never: None,
                unless_trusted: None,
                granular: None,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut model = model_info_from_slug("unknown-model");
        model.model_messages = Some(messages.clone());

        let updated = with_config_overrides(model, &config);

        messages.instructions_template = Some(expected.to_string());
        assert_eq!(updated.model_messages, Some(messages));
    }
}

#[test]
fn baked_personality_section_is_preserved_without_explicit_none() {
    let instructions = "Intro\n# Personality\nKeep me\n# General\nKeep me too";
    let configs = [
        config_with_personality(/*personality*/ None),
        config_with_personality(Some(Personality::Friendly)),
        config_with_personality(Some(Personality::Pragmatic)),
    ];

    for config in configs {
        let mut model = model_info_from_slug("unknown-model");
        model
            .model_messages
            .as_mut()
            .expect("fallback model messages")
            .instructions_template = Some(instructions.to_string());

        assert_eq!(
            render_model_instructions(&with_config_overrides(model, &config)),
            instructions
        );
    }
}

#[test]
fn explicit_empty_base_instructions_stay_empty_with_personality_none() {
    let mut model = model_info_from_slug("unknown-model");
    model
        .model_messages
        .as_mut()
        .expect("fallback model messages")
        .instructions_template = Some("Intro\n# Personality\nRemove me".to_string());
    let config = ModelsManagerConfig {
        base_instructions: Some(String::new()),
        personality: Some(Personality::None),
        ..Default::default()
    };

    let updated = with_config_overrides(model, &config);

    assert_eq!(
        updated
            .model_messages
            .as_ref()
            .and_then(|messages| messages.instructions_template.as_deref()),
        Some("")
    );
    assert_eq!(render_model_instructions(&updated), "");
}

#[test]
fn unknown_model_uses_builtin_instruction_template() {
    let model = model_info_from_slug("unknown-model");

    assert_eq!(render_model_instructions(&model), BASE_INSTRUCTIONS);
}

#[test]
fn model_context_window_override_clamps_to_max_context_window() {
    let mut model = model_info_from_slug("unknown-model");
    model.context_window = Some(273_000);
    model.max_context_window = Some(400_000);
    let config = ModelsManagerConfig {
        model_context_window: Some(500_000),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.context_window = Some(400_000);

    assert_eq!(updated, expected);
}

#[test]
fn model_context_window_uses_model_value_without_override() {
    let mut model = model_info_from_slug("unknown-model");
    model.context_window = Some(273_000);
    model.max_context_window = Some(400_000);
    let config = ModelsManagerConfig::default();

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn model_context_v1_window_precedence_and_legacy_golden() {
    // (row, Profile, maximum, legacy result, v1 result).
    for (row, profile, max, legacy, v1) in [
        (Some(400), Some(800), Some(1000), Some(800), Some(800)),
        (None, Some(800), Some(1000), Some(800), Some(800)),
        (None, None, Some(1000), Some(1000), Some(1000)),
        (Some(1400), None, Some(1000), Some(1400), Some(1000)),
        (Some(400), Some(1600), Some(1000), Some(1000), Some(1000)),
        (None, Some(1600), Some(1000), Some(1000), Some(1000)),
        (None, None, None, None, None),
    ] {
        for (enabled, expected) in [(false, legacy), (true, v1)] {
            let mut model = model_info_from_slug("window-test");
            model.context_window = row;
            model.max_context_window = max;
            let config = ModelsManagerConfig {
                model_context_settings_v1: enabled,
                model_context_window: profile,
                ..Default::default()
            };
            let resolved = with_config_overrides(model, &config);
            assert_eq!(
                resolved.resolved_context_window(),
                expected,
                "row={row:?}, profile={profile:?}, max={max:?}, v1={enabled}"
            );
        }
    }
}

#[test]
fn model_context_v1_compaction_precedence_and_legacy_golden() {
    // (row absolute, row percent, Profile absolute, Profile percent, legacy, v1).
    for (row_abs, row_percent, profile_abs, profile_percent, legacy, v1) in [
        (Some(700), None, Some(200), None, 200, 200),
        (None, Some(75), Some(200), None, 200, 200),
        (Some(700), None, None, Some(80), 700, 800),
        (None, Some(75), None, Some(80), 900, 800),
        (None, None, None, Some(80), 900, 800),
        (None, None, None, None, 900, 900),
        (None, Some(100), None, None, 900, 900),
        (Some(1200), None, None, None, 900, 900),
    ] {
        for (enabled, expected) in [(false, legacy), (true, v1)] {
            let mut model = model_info_from_slug("percent-test");
            model.context_window = Some(1000);
            model.max_context_window = Some(1000);
            model.effective_context_window_percent = 95;
            model.auto_compact_token_limit = row_abs;
            model.auto_compact_percent = row_percent;
            let config = ModelsManagerConfig {
                model_context_settings_v1: enabled,
                model_auto_compact_token_limit: profile_abs,
                model_auto_compact_percent: profile_percent,
                ..Default::default()
            };
            let resolved = with_config_overrides(model, &config);
            assert_eq!(resolved.usable_context_window(), Some(950));
            assert_eq!(
                resolved.auto_compact_token_limit(),
                Some(expected),
                "row={row_abs:?}/{row_percent:?}, profile={profile_abs:?}/{profile_percent:?}, v1={enabled}"
            );
        }
    }
}

#[test]
fn model_context_v1_unknown_models_do_not_invent_catalog_windows() {
    for (enabled, profile, expected) in [
        (false, None, Some(272_000)),
        (false, Some(800_000), Some(272_000)),
        (false, Some(500_000), Some(272_000)),
        (true, None, None),
        (true, Some(800_000), Some(800_000)),
        (true, Some(500_000), Some(500_000)),
    ] {
        let resolved = crate::manager::construct_model_info_from_candidates(
            "unknown",
            &[],
            crate::ModelOrigin::ExplicitConfig,
            &ModelsManagerConfig {
                model_context_settings_v1: enabled,
                model_context_window: profile,
                model_auto_compact_percent: Some(80),
                ..Default::default()
            },
        );
        assert_eq!(resolved.resolved_context_window(), expected);
        assert_eq!(
            resolved.auto_compact_token_limit(),
            Some(expected.unwrap_or(272_000) * if enabled { 80 } else { 90 } / 100)
        );
    }
}

#[test]
fn model_context_v1_shared_migration_golden_vectors() {
    for (profile_w, profile_a, row_w, max, row_a, expected_w, expected_a) in [
        (
            Some(200000),
            Some(150000),
            128000,
            256000,
            Some(90000),
            200000,
            150000,
        ),
        (
            Some(500000),
            Some(400000),
            128000,
            256000,
            None,
            256000,
            230400,
        ),
        (
            None,
            Some(500000),
            128000,
            256000,
            Some(90000),
            128000,
            115200,
        ),
        (
            Some(512000),
            None,
            1000000,
            1000000,
            Some(700000),
            512000,
            460800,
        ),
    ] {
        let mut row = model_info_from_slug("migration-row");
        row.context_window = Some(row_w);
        row.max_context_window = Some(max);
        row.auto_compact_token_limit = row_a;
        let mut config = ModelsManagerConfig {
            model_context_window: profile_w,
            model_auto_compact_token_limit: profile_a,
            ..Default::default()
        };
        let legacy = with_config_overrides(row.clone(), &config);
        assert_eq!(
            (
                legacy.resolved_context_window(),
                legacy.auto_compact_token_limit()
            ),
            (Some(expected_w), Some(expected_a))
        );
        // B: the original numbers are catalog built-ins, not owner or preservation pins.
        config.model_context_settings_v1 = true;
        let migrated = with_config_overrides(row, &config);
        let provenance = migrated.context_settings.as_ref().unwrap();
        assert!(provenance.owner.is_none());
        assert!(provenance.preserved.is_none());
        assert_eq!(
            provenance.catalog.as_ref().unwrap().context_window,
            Some(row_w)
        );
        assert_eq!(
            (
                migrated.resolved_context_window(),
                migrated.auto_compact_token_limit()
            ),
            (Some(expected_w), Some(expected_a))
        );
    }
}

#[test]
fn model_context_v1_catalog_lookup_keeps_prefix_namespace_and_variant_identity() {
    let mut base = model_info_from_slug("gpt-6");
    base.context_window = Some(100_000);
    base.max_context_window = Some(1_000_000);
    base.auto_compact_percent = Some(50);
    let mut specific = base.clone();
    specific.slug = "gpt-6.1".into();
    specific.context_window = Some(200_000);
    let mut variant = base.clone();
    variant.slug = "custom".into();
    variant.request_model = Some("gpt-6.1".into());
    variant.context_window = Some(300_000);
    for (slug, window, known) in [
        ("gpt-6.1-preview", 200_000, true),
        ("provider/gpt-6.1-preview", 200_000, true),
        ("custom", 300_000, true),
        ("custom-preview", 500_000, false),
        ("provider/custom", 500_000, false),
        ("provider/inner/gpt-6.1", 500_000, false),
    ] {
        let config = ModelsManagerConfig {
            model_context_settings_v1: true,
            model_context_window: Some(500_000),
            model_auto_compact_token_limit: Some(400_000),
            ..Default::default()
        };
        let mut catalog = [base.clone(), specific.clone(), variant.clone()];
        for row in &mut catalog {
            set_owner_context_for_test(row);
        }
        let resolved = crate::manager::construct_model_info_from_candidates(
            slug,
            &catalog,
            crate::ModelOrigin::ExplicitConfig,
            &config,
        );
        assert_eq!(resolved.resolved_context_window(), Some(window), "{slug}");
        assert_eq!(
            resolved.auto_compact_token_limit(),
            Some(if known { window / 2 } else { 400_000 }),
            "{slug}"
        );
        assert_eq!(
            resolved.origin == crate::ModelOrigin::ExplicitConfig,
            known,
            "{slug}"
        );
    }
}

#[test]
fn model_context_v1_shared_percent_vectors_use_the_resolved_nominal_window() {
    for (row_percent, profile_percent, profile_absolute, expected) in [
        (Some(50), None, Some(150_000), 64_000),
        (None, Some(75), None, 96_000),
        (Some(100), None, Some(150_000), 115_200),
    ] {
        let mut row = model_info_from_slug("percent-vector");
        row.context_window = Some(128_000);
        row.max_context_window = Some(256_000);
        row.auto_compact_percent = row_percent;
        set_owner_context_for_test(&mut row);
        let config = ModelsManagerConfig {
            model_context_settings_v1: true,
            model_context_window: Some(200_000),
            model_auto_compact_percent: profile_percent,
            model_auto_compact_token_limit: profile_absolute,
            ..Default::default()
        };
        let resolved = with_config_overrides(row, &config);
        assert_eq!(resolved.resolved_context_window(), Some(128_000));
        assert_eq!(resolved.auto_compact_token_limit(), Some(expected));
    }
}

#[test]
fn model_context_v1_unknown_budget_fallback_preserves_compaction_without_capping_profile() {
    for (window, percent, absolute, expected) in [
        (None, None, None, 244_800),
        (None, Some(50), None, 136_000),
        (None, Some(100), None, 244_800),
        (None, None, Some(500_000), 244_800),
        (Some(500_000), None, None, 450_000),
        (Some(500_000), Some(50), None, 250_000),
    ] {
        let config = ModelsManagerConfig {
            model_context_settings_v1: true,
            model_context_window: window,
            model_auto_compact_percent: percent,
            model_auto_compact_token_limit: absolute,
            ..Default::default()
        };
        let resolved = crate::manager::construct_model_info_from_candidates(
            "custom-subagent",
            &[],
            crate::ModelOrigin::ExplicitConfig,
            &config,
        );
        assert_eq!(resolved.resolved_context_window(), window);
        assert_eq!(
            resolved.budget_context_window(),
            Some(window.unwrap_or(272_000))
        );
        assert_eq!(resolved.auto_compact_token_limit(), Some(expected));
        assert_eq!(resolved.max_context_window, None);
    }
}

#[test]
fn model_context_v1_budget_hint_does_not_leak_into_legacy_resolution() {
    let mut model = model_info_from_slug("unknown");
    model.context_window = None;
    model.max_context_window = None;
    model.runtime_context_window_fallback = Some(272_000);
    let legacy = with_config_overrides(model, &ModelsManagerConfig::default());
    assert_eq!(legacy.budget_context_window(), None);
    assert_eq!(legacy.auto_compact_token_limit(), None);
}

fn set_owner_context_for_test(row: &mut ModelInfo) {
    use codex_protocol::openai_models::{ModelContextSettings, ModelContextSettingsValues};
    row.context_settings = Some(ModelContextSettings {
        owner: Some(ModelContextSettingsValues {
            context_window: row.context_window,
            auto_compact_percent: row.auto_compact_percent,
            auto_compact_token_limit: row.auto_compact_token_limit,
        }),
        ..Default::default()
    });
}

#[test]
fn model_context_v2_window_precedence_steps_and_max_cap() {
    for (preserved, owner, profile, catalog, max, expected) in [
        (
            Some(600),
            Some(500),
            Some(400),
            Some(300),
            Some(1000),
            Some(600),
        ),
        (None, Some(500), Some(400), Some(300), Some(1000), Some(500)),
        (None, None, Some(400), Some(300), Some(1000), Some(400)),
        (None, None, None, Some(300), Some(1000), Some(300)),
        (None, None, None, None, Some(1000), Some(1000)),
        (None, None, None, None, None, None),
        (
            Some(2000),
            Some(500),
            Some(400),
            Some(300),
            Some(1000),
            Some(1000),
        ),
        (None, None, Some(2000), Some(300), Some(1000), Some(1000)),
    ] {
        let mut row = model_info_from_slug("layered");
        row.context_window = Some(111); // Flat compatibility data is not another owner layer.
        row.max_context_window = max;
        row.context_settings = Some(ModelContextSettings {
            catalog: Some(ModelContextSettingsValues {
                context_window: catalog,
                ..Default::default()
            }),
            owner: Some(ModelContextSettingsValues {
                context_window: owner,
                ..Default::default()
            }),
            preserved: Some(ModelContextSettingsValues {
                context_window: preserved,
                ..Default::default()
            }),
        });
        let original = row.context_settings.clone();
        let resolved = with_config_overrides(
            row,
            &ModelsManagerConfig {
                model_context_settings_v1: true,
                model_context_window: profile,
                ..Default::default()
            },
        );
        assert_eq!(resolved.resolved_context_window(), expected);
        assert_eq!(resolved.context_settings, original);
    }
}

#[test]
fn model_context_v2_compaction_category_is_atomic_at_every_precedence_step() {
    fn pct(p: u8) -> ModelContextSettingsValues {
        ModelContextSettingsValues {
            auto_compact_percent: Some(p),
            ..Default::default()
        }
    }
    fn abs(a: i64) -> ModelContextSettingsValues {
        ModelContextSettingsValues {
            auto_compact_token_limit: Some(a),
            ..Default::default()
        }
    }
    for (preserved, owner, profile, catalog, selected, expected) in [
        (
            Some(pct(50)),
            Some(abs(200)),
            Some(pct(80)),
            Some(abs(300)),
            Some(pct(50)),
            500,
        ),
        (
            Some(abs(450)),
            Some(pct(60)),
            Some(abs(200)),
            Some(pct(75)),
            Some(abs(450)),
            450,
        ),
        (
            None,
            Some(pct(60)),
            Some(abs(200)),
            Some(pct(75)),
            Some(pct(60)),
            600,
        ),
        (
            None,
            Some(abs(350)),
            Some(pct(80)),
            Some(abs(200)),
            Some(abs(350)),
            350,
        ),
        (
            None,
            None,
            Some(pct(80)),
            Some(abs(200)),
            Some(pct(80)),
            800,
        ),
        (
            None,
            None,
            Some(abs(100)),
            Some(pct(75)),
            Some(abs(100)),
            100,
        ),
        (None, None, None, Some(pct(75)), Some(pct(75)), 750),
        (None, None, None, None, None, 900),
        (Some(pct(100)), None, None, None, Some(pct(100)), 900),
        (None, Some(abs(1500)), None, None, Some(abs(1500)), 900),
    ] {
        let mut row = model_info_from_slug("layered");
        row.context_window = Some(1000);
        row.max_context_window = Some(1000);
        row.context_settings = Some(ModelContextSettings {
            catalog: Some(ModelContextSettingsValues {
                context_window: Some(1000),
                ..catalog.unwrap_or_default()
            }),
            owner,
            preserved,
        });
        let profile = profile.unwrap_or_default();
        let resolved = with_config_overrides(
            row,
            &ModelsManagerConfig {
                model_context_settings_v1: true,
                model_auto_compact_percent: profile.auto_compact_percent,
                model_auto_compact_token_limit: profile.auto_compact_token_limit,
                ..Default::default()
            },
        );
        let selected = selected.unwrap_or_default();
        assert_eq!(
            (
                resolved.auto_compact_percent,
                resolved.auto_compact_token_limit
            ),
            (
                selected.auto_compact_percent,
                selected.auto_compact_token_limit
            )
        );
        assert_eq!(resolved.auto_compact_token_limit(), Some(expected));
    }
}

#[test]
fn model_context_v2_preservation_restore_and_independent_fields() {
    let mut row = model_info_from_slug("conflict");
    row.max_context_window = Some(256_000);
    row.context_settings = Some(ModelContextSettings {
        catalog: Some(ModelContextSettingsValues {
            context_window: Some(128_000),
            auto_compact_token_limit: Some(90_000),
            ..Default::default()
        }),
        owner: Some(ModelContextSettingsValues {
            context_window: Some(128_000),
            auto_compact_token_limit: Some(90_000),
            ..Default::default()
        }),
        preserved: Some(ModelContextSettingsValues {
            context_window: Some(200_000),
            auto_compact_token_limit: Some(150_000),
            ..Default::default()
        }),
    });
    let mut config = ModelsManagerConfig {
        model_context_settings_v1: true,
        model_context_window: Some(200_000),
        model_auto_compact_token_limit: Some(150_000),
        ..Default::default()
    };
    for (stage, expected_window, expected_absolute) in [
        (0, 200_000, 150_000),
        (1, 200_000, 150_000),
        (2, 128_000, 90_000),
        (3, 256_000, 128_000),
        (4, 128_000, 90_000),
    ] {
        match stage {
            1 => {
                config.model_context_window = Some(400_000);
                config.model_auto_compact_token_limit = None;
                config.model_auto_compact_percent = Some(50);
            }
            2 => row.context_settings.as_mut().unwrap().preserved = None,
            3 => row.context_settings.as_mut().unwrap().owner = None,
            4 => {
                config.model_context_window = None;
                config.model_auto_compact_percent = None;
            }
            _ => {}
        }
        let resolved = with_config_overrides(row.clone(), &config);
        assert_eq!(
            (
                resolved.resolved_context_window(),
                resolved.auto_compact_token_limit()
            ),
            (Some(expected_window), Some(expected_absolute)),
            "stage{stage}"
        );
    }
    // A preserved compaction category does not shadow the owner's window.
    row.context_settings.as_mut().unwrap().owner = Some(ModelContextSettingsValues {
        context_window: Some(180_000),
        ..Default::default()
    });
    row.context_settings.as_mut().unwrap().preserved = Some(ModelContextSettingsValues {
        auto_compact_percent: Some(50),
        ..Default::default()
    });
    let resolved = with_config_overrides(row, &config);
    assert_eq!(
        (
            resolved.resolved_context_window(),
            resolved.auto_compact_token_limit()
        ),
        (Some(180_000), Some(90_000))
    );
}

#[test]
fn model_context_v2_flat_catalog_round_trip_retains_original_defaults() {
    let mut row = model_info_from_slug("flat");
    row.context_window = Some(128_000);
    row.max_context_window = Some(256_000);
    row.auto_compact_token_limit = Some(90_000);
    let mut config = ModelsManagerConfig {
        model_context_settings_v1: true,
        model_context_window: Some(200_000),
        model_auto_compact_token_limit: Some(150_000),
        ..Default::default()
    };
    let resolved = with_config_overrides(row, &config);
    let wire = serde_json::to_value(&resolved).unwrap();
    assert_eq!(
        wire["context_settings"]["catalog"]["context_window"],
        128_000
    );
    assert!(wire["context_settings"].get("owner").is_none());
    let decoded: ModelInfo = serde_json::from_value(wire).unwrap();
    config.model_context_window = None;
    config.model_auto_compact_token_limit = None;
    let inherited = with_config_overrides(decoded, &config);
    assert_eq!(
        (
            inherited.resolved_context_window(),
            inherited.auto_compact_token_limit()
        ),
        (Some(128_000), Some(90_000))
    );
}

#[test]
fn model_context_v2_window_and_compaction_preservation_inherit_independently() {
    let mut row = model_info_from_slug("independent");
    row.max_context_window = Some(256_000);
    row.context_settings = Some(ModelContextSettings {
        catalog: Some(ModelContextSettingsValues {
            context_window: Some(128_000),
            auto_compact_token_limit: Some(90_000),
            ..Default::default()
        }),
        owner: Some(ModelContextSettingsValues {
            context_window: Some(128_000),
            ..Default::default()
        }),
        preserved: Some(ModelContextSettingsValues {
            context_window: Some(200_000),
            ..Default::default()
        }),
    });
    let mut config = ModelsManagerConfig {
        model_context_settings_v1: true,
        model_context_window: Some(200_000),
        model_auto_compact_token_limit: Some(150_000),
        ..Default::default()
    };
    let before = with_config_overrides(row.clone(), &config);
    assert_eq!(
        (
            before.resolved_context_window(),
            before.auto_compact_token_limit()
        ),
        (Some(200_000), Some(150_000))
    );
    config.model_context_window = Some(400_000);
    config.model_auto_compact_token_limit = None;
    config.model_auto_compact_percent = Some(50);
    let after = with_config_overrides(row.clone(), &config);
    assert_eq!(
        (
            after.resolved_context_window(),
            after.auto_compact_token_limit()
        ),
        (Some(200_000), Some(100_000))
    );
    // A preserved compaction value leaves the window inherited from Profile.
    row.context_settings.as_mut().unwrap().owner = Some(ModelContextSettingsValues {
        auto_compact_token_limit: Some(90_000),
        ..Default::default()
    });
    row.context_settings.as_mut().unwrap().preserved = Some(ModelContextSettingsValues {
        auto_compact_token_limit: Some(150_000),
        ..Default::default()
    });
    let preserved = with_config_overrides(row.clone(), &config);
    assert_eq!(
        (
            preserved.resolved_context_window(),
            preserved.auto_compact_token_limit()
        ),
        (Some(256_000), Some(150_000))
    );
    row.context_settings.as_mut().unwrap().preserved = None;
    let restored = with_config_overrides(row, &config);
    assert_eq!(
        (
            restored.resolved_context_window(),
            restored.auto_compact_token_limit()
        ),
        (Some(256_000), Some(90_000))
    );
}
