use anyhow::Result;
use codex_core::TurnInputRequest;
use codex_login::CodexAuth;
use codex_models_manager::manager::RefreshStrategy;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::user_input::UserInput;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use test_case::test_case;

#[test_case(100; "within_both_limits")]
#[test_case(280_000; "downshift_compacts_before_next_turn")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_variants_switch_context_and_compact_using_upstream_model(
    first_turn_tokens: i64,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let server = responses::start_mock_server().await;
    let summary = responses::sse(vec![
        serde_json::json!({"type": "response.output_item.done", "item": {"type": "compaction", "encrypted_content": "variant-compaction-summary"}}),
        responses::ev_completed_with_tokens("compact", /*total_tokens*/ 20),
    ]);
    let mut replies = vec![responses::sse(vec![responses::ev_completed_with_tokens(
        "r1",
        first_turn_tokens,
    )])];
    if first_turn_tokens > 240_000 {
        replies.push(summary.clone());
    }
    replies.push(responses::sse(vec![responses::ev_completed_with_tokens(
        "r2", /*total_tokens*/ 120,
    )]));
    replies.push(summary);
    let expected_requests = replies.len();
    let requests = responses::mount_response_sequence(
        &server,
        replies
            .into_iter()
            .map(|reply| {
                responses::sse_response(reply).insert_header("OpenAI-Model", "gpt-6-astra")
            })
            .collect(),
    )
    .await;
    let test = test_codex()
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_model_info_override("Astra [400K]", |model| {
            model.request_model = Some("gpt-6-astra".to_string());
            model.supported_in_api = false;
            model.context_window = Some(400_000);
            model.max_context_window = Some(1_000_000);
            model.auto_compact_token_limit = Some(350_000);
            model.tool_mode = None;
            model.multi_agent_version = None;
            model.use_responses_lite = false;
            model.comp_hash = None;
        })
        .with_model_info_override("Astra [285K]", |model| {
            model.request_model = Some("gpt-6-astra".to_string());
            model.supported_in_api = false;
            model.context_window = Some(285_000);
            model.max_context_window = Some(1_000_000);
            model.auto_compact_token_limit = Some(240_000);
            model.tool_mode = None;
            model.multi_agent_version = None;
            model.use_responses_lite = false;
            model.comp_hash = None;
        })
        .with_model("Astra [400K]")
        .with_config(|config| {
            config.model_provider.name = "OpenAI".to_string();
            config.model_provider.supports_websockets = false;
        })
        .build_with_auto_env(&server)
        .await?;
    let manager = test.thread_manager.get_models_manager();
    let available = manager
        .list_models(
            RefreshStrategy::Offline,
            codex_core::test_support::default_http_client_factory(),
        )
        .await;
    for (slug, context, compact) in [
        ("Astra [400K]", 400_000, 350_000),
        ("Astra [285K]", 285_000, 240_000),
    ] {
        assert!(available.iter().any(|model| model.model == slug));
        let info = manager
            .get_model_info(slug, &test.config.to_models_manager_config())
            .await;
        assert_eq!(
            (
                info.slug.as_str(),
                info.request_model(),
                info.resolved_context_window(),
                info.auto_compact_token_limit()
            ),
            (slug, "gpt-6-astra", Some(context), Some(compact))
        );
        test.codex
            .start_or_steer_turn(
                TurnInputRequest::user_input(vec![UserInput::Text {
                    text: "hello".to_string(),
                    text_elements: Vec::new(),
                }])
                .with_thread_settings(ThreadSettingsOverrides {
                    model: Some(slug.to_string()),
                    ..Default::default()
                }),
            )
            .await?;
        let event = wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnStarted(_))
        })
        .await;
        let EventMsg::TurnStarted(started) = event else {
            unreachable!()
        };
        assert_eq!(started.model_context_window, Some(context * 95 / 100));
        loop {
            match wait_for_event(&test.codex, |_| true).await {
                EventMsg::ModelReroute(event) => {
                    panic!("upstream model must not reroute its local variant: {event:?}")
                }
                EventMsg::Error(event) => panic!("variant turn failed: {event:?}"),
                EventMsg::TurnComplete(_) => break,
                _ => {}
            }
        }
    }
    test.codex.submit(Op::Compact).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(
        requests
            .requests()
            .iter()
            .map(|request| request.body_json()["model"].clone())
            .collect::<Vec<_>>(),
        vec![serde_json::json!("gpt-6-astra"); expected_requests]
    );
    Ok(())
}
