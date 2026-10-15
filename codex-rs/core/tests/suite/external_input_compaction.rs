//! Mid-turn compaction keeps the running turn's ExternalInput.
//!
//! cutex delivers asks and agent messages as canonical `external_event` items. Compaction used
//! to keep only user messages, so after a mid-turn compaction the model's latest input was an
//! older user message and it answered that instead of the current ask.
use anyhow::Result;
use codex_core::context::CanonicalBytePolicy;
use codex_login::CodexAuth;
use codex_model_provider_info::built_in_model_providers;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Message;
use codex_protocol::external_input::Source;
use codex_protocol::external_input::SourceKind;
use codex_protocol::protocol::EventMsg;
use core_test_support::responses::ResponsesRequest;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_completed_with_tokens;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_response_sequence;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::sse_response;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodexBuilder;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;
use wiremock::MockServer;
use wiremock::ResponseTemplate;

const OWNER: &str = "cutex-owner";
const OLD_REQUEST: &str = "OLD_OWNER_REQUEST";
const ASK_TEXT: &str = "CURRENT_TURN_ASK";

fn envelope(thread_id: &str) -> Envelope {
    let mut envelope = Envelope {
        version: 1,
        owner_id: OWNER.to_string(),
        thread_id: thread_id.to_string(),
        runtime_generation: 1,
        message: Message {
            id: "ask-1".to_string(),
            source: Source {
                kind: SourceKind::Agent,
                id: "requester".to_string(),
            },
            event_type: "ASK".to_string(),
            delivery: Delivery::AfterTurn,
            text: ASK_TEXT.to_string(),
        },
        semantic_sha256: String::new(),
        view: None,
    };
    envelope.semantic_sha256 = envelope.digest();
    envelope
}

fn positions(input: &[Value], needle: &str) -> Vec<usize> {
    input
        .iter()
        .enumerate()
        .filter(|(_, item)| item.to_string().contains(needle))
        .map(|(index, _)| index)
        .collect()
}

/// Runs an owner turn, rebinds the thread to the cutex owner, then delivers an ask whose turn
/// compacts mid-turn. Returns the requests of the ask turn.
async fn ask_turn_with_mid_turn_compaction(
    server: &MockServer,
    builder: impl Fn() -> TestCodexBuilder,
    ask_turn: Vec<ResponseTemplate>,
) -> Result<Vec<ResponsesRequest>> {
    // An older owner turn leaves a real user message in history.
    let initial = builder().build(server).await?;
    mount_sse_once(
        server,
        sse(vec![
            ev_response_created("old"),
            ev_assistant_message("old-message", "done"),
            ev_completed("old"),
        ]),
    )
    .await;
    initial.submit_text_turn(OLD_REQUEST).await?;
    let thread_id = initial.session_configured.thread_id.to_string();
    let bound_thread = thread_id.clone();
    let test = builder()
        .with_thread_manager(move |manager| {
            manager.with_external_input_owner(
                OWNER.to_string(),
                bound_thread,
                CanonicalBytePolicy::default(),
            )
        })
        .restart(server, &initial)
        .await?;

    let responses = mount_response_sequence(server, ask_turn).await;
    test.codex
        .submit_external_input_exclusive(envelope(&thread_id))
        .await?
        .expect("exclusive admission starts a turn");
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    Ok(responses.requests())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_mid_turn_compaction_keeps_the_current_external_ask() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let builder = || {
        test_codex()
            .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
            .with_config(|config| {
                config.model_auto_compact_token_limit = Some(200);
            })
    };
    let requests = ask_turn_with_mid_turn_compaction(
        &server,
        builder,
        vec![
            sse_response(sse(vec![
                ev_function_call("call-before-compact", "test_tool", "{}"),
                ev_completed_with_tokens("r1", /*total_tokens*/ 500),
            ])),
            sse_response(sse(vec![
                json!({
                    "type": "response.output_item.done",
                    "item": {
                        "type": "compaction",
                        "encrypted_content": "V2_COMPACT_SUMMARY",
                    }
                }),
                ev_completed("r-compact"),
            ])),
            sse_response(sse(vec![
                ev_assistant_message("m1", "FINAL_REPLY"),
                ev_completed_with_tokens("r2", /*total_tokens*/ 80),
            ])),
        ],
    )
    .await?;

    assert_eq!(requests.len(), 3);
    assert!(
        requests[1]
            .body_json()
            .to_string()
            .contains("\"type\":\"compaction_trigger\""),
        "second request is the remote compaction"
    );
    let continuation = requests[2].input();
    let compaction = positions(&continuation, "V2_COMPACT_SUMMARY");
    let old = positions(&continuation, OLD_REQUEST);
    let ask = positions(&continuation, ASK_TEXT);
    assert_eq!(compaction.len(), 1, "{continuation:#?}");
    assert_eq!(old.len(), 1, "{continuation:#?}");
    assert_eq!(
        ask.len(),
        1,
        "the ask survives compaction once: {continuation:#?}"
    );
    assert!(
        old[0] < ask[0] && ask[0] < compaction[0],
        "the ask is the latest input before the compaction item: {continuation:#?}"
    );
    assert_eq!(
        continuation[ask[0]]["namespace"].as_str(),
        Some("external"),
        "the canonical external item is kept verbatim: {continuation:#?}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_mid_turn_compaction_keeps_the_current_external_ask() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let mut provider = built_in_model_providers(/*openai_base_url*/ None)["openai"].clone();
    provider.name = "OpenAI (test)".into();
    provider.base_url = Some(format!("{}/v1", server.uri()));
    provider.supports_websockets = false;
    let builder = move || {
        let provider = provider.clone();
        test_codex().with_config(move |config| {
            config.model_provider = provider;
            config.model_context_window = Some(100);
            config.model_auto_compact_token_limit = Some(90);
        })
    };
    let requests = ask_turn_with_mid_turn_compaction(
        &server,
        builder,
        vec![
            sse_response(sse(vec![
                ev_function_call("call-before-compact", "test_tool", "{}"),
                ev_completed_with_tokens("r1", /*total_tokens*/ 96),
            ])),
            sse_response(sse(vec![
                ev_assistant_message("summary", "LOCAL_COMPACT_SUMMARY"),
                ev_completed_with_tokens("r-compact", /*total_tokens*/ 10),
            ])),
            sse_response(sse(vec![
                ev_assistant_message("m1", "FINAL_REPLY"),
                ev_completed_with_tokens("r2", /*total_tokens*/ 10),
            ])),
        ],
    )
    .await?;

    assert_eq!(requests.len(), 3);
    let continuation = requests[2].input();
    let summary = positions(&continuation, "LOCAL_COMPACT_SUMMARY");
    let old = positions(&continuation, OLD_REQUEST);
    let ask = positions(&continuation, ASK_TEXT);
    assert_eq!(summary.len(), 1, "{continuation:#?}");
    assert_eq!(old.len(), 1, "{continuation:#?}");
    assert_eq!(
        ask.len(),
        1,
        "the ask survives compaction once: {continuation:#?}"
    );
    assert!(
        old[0] < ask[0] && ask[0] < summary[0],
        "the ask is the latest input before the summary: {continuation:#?}"
    );
    Ok(())
}
