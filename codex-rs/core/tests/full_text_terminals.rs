//! Provider-stream tests execute no helper binaries and need no arg0 aliases.
#![allow(clippy::expect_used)]

use codex_core::TurnInputRequest;
use codex_history::RolloutItem;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadHistoryMode;
use codex_protocol::user_input::UserInput;
use codex_thread_store::LoadThreadHistoryParams;
use core_test_support::responses::*;
use core_test_support::streaming_sse::StreamingSseChunk;
use core_test_support::streaming_sse::start_streaming_sse_server;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use futures::future::LocalBoxFuture;
use pretty_assertions::assert_eq;
use serde_json::json;
use tokio::sync::oneshot;

#[derive(Clone, Copy)]
enum Ending {
    Cancel,
    Error,
    Eof,
    Complete,
    CompleteWithoutRawContent,
    CompleteEmptyRawContent,
}

#[inline(never)]
fn full_text_terminal(
    ending: Ending,
    history_mode: ThreadHistoryMode,
) -> LocalBoxFuture<'static, anyhow::Result<()>> {
    Box::pin(async move {
        let (gate_tx, gate_rx) = oneshot::channel();
        let mut summary_second = ev_reasoning_summary_text_delta("第二段");
        summary_second["summary_index"] = json!(1);
        let mut raw_second = ev_reasoning_text_delta("raw second");
        raw_second["content_index"] = json!(1);
        let first = sse(vec![
            ev_response_created("response-partial"),
            ev_message_item_added("msg-first", ""),
            ev_output_text_delta("first 中文🙂"),
            ev_assistant_message("msg-first", "first 中文🙂"),
            ev_reasoning_item_added("reasoning-partial", &[]),
            ev_reasoning_summary_text_delta("第一段"),
            summary_second,
            ev_reasoning_text_delta("raw first"),
            raw_second,
            ev_message_item_added("msg-last", ""),
            ev_output_text_delta("last 中文🙂"),
        ]);
        let tail = match ending {
            Ending::Complete => sse(vec![
                ev_assistant_message("msg-last", "last 中文🙂"),
                ev_reasoning_item(
                    "reasoning-partial",
                    &["第一段", "第二段"],
                    &["raw first", "raw second"],
                ),
                ev_completed("response-partial"),
            ]),
            Ending::CompleteWithoutRawContent => {
                let mut reasoning = ev_reasoning_item(
                    "reasoning-partial",
                    &["第一段", "第二段"],
                    &["raw first", "raw second"],
                );
                reasoning["item"]
                    .as_object_mut()
                    .expect("fixture reasoning item")
                    .remove("content");
                sse(vec![
                    ev_assistant_message("msg-last", "last 中文🙂"),
                    reasoning,
                    ev_completed("response-partial"),
                ])
            }
            Ending::CompleteEmptyRawContent => {
                let mut reasoning = ev_reasoning_item(
                    "reasoning-partial",
                    &["第一段", "第二段"],
                    &["raw first", "raw second"],
                );
                reasoning["item"]["content"] = json!([]);
                sse(vec![
                    ev_assistant_message("msg-last", "last 中文🙂"),
                    reasoning,
                    ev_completed("response-partial"),
                ])
            }
            Ending::Error => sse(vec![json!({"type":"response.failed", "response": {
                "id":"response-partial", "error":{"code":"invalid_prompt", "message":"fixture failure"}
            }})]),
            Ending::Cancel | Ending::Eof => String::new(),
        };
        let followup = vec![StreamingSseChunk {
            gate: None,
            body: sse(vec![
                ev_response_created("response-followup"),
                ev_assistant_message("msg-followup", "done"),
                ev_completed("response-followup"),
            ]),
        }];
        let (server, _) = start_streaming_sse_server(vec![
            vec![
                StreamingSseChunk {
                    gate: None,
                    body: first,
                },
                StreamingSseChunk {
                    gate: Some(gate_rx),
                    body: tail,
                },
            ],
            followup,
        ])
        .await;
        let test = test_codex()
            .with_model("gpt-5.4")
            .with_history_mode(history_mode)
            .with_config(|config| {
                config.model_provider.stream_max_retries = Some(0);
                config.model_provider.request_max_retries = Some(0);
            })
            .build_with_streaming_server(&server)
            .await?;
        test.codex
            .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
                text: "stream fixture".into(),
                text_elements: vec![],
            }]))
            .await?;
        wait_for_event(&test.codex, |event| {
            matches!(event,
        EventMsg::AgentMessageContentDelta(delta) if delta.item_id == "msg-last")
        })
        .await;
        if matches!(ending, Ending::Cancel) {
            test.codex.submit(Op::Interrupt).await?;
        } else {
            let _ = gate_tx.send(());
        }
        let mut completions = vec![];
        loop {
            let event = test.codex.next_event().await?;
            match event.msg {
                EventMsg::ItemCompleted(completed) => completions.push(completed.item),
                EventMsg::TurnComplete(_) | EventMsg::TurnAborted(_) => break,
                _ => {}
            }
        }
        assert_eq!(
            completions.len(),
            2,
            "both open items close, already completed items stay closed"
        );
        let reasoning_item = completions
            .iter()
            .find(|item| item.id() == "reasoning-partial")
            .expect("reasoning terminal");
        let TurnItem::Reasoning(reasoning) = reasoning_item else {
            panic!("expected reasoning terminal")
        };
        let message = completions
            .iter()
            .find(|item| item.id() == "msg-last")
            .expect("message terminal");
        assert_eq!(
            serde_json::to_value(message)?["content"][0]["text"],
            "last 中文🙂"
        );
        let expected_raw = if matches!(ending, Ending::CompleteEmptyRawContent) {
            vec![]
        } else {
            vec!["raw first".to_string(), "raw second".to_string()]
        };
        assert_eq!(
            (
                &reasoning.id,
                &reasoning.summary_text,
                &reasoning.raw_content
            ),
            (
                &"reasoning-partial".to_string(),
                &vec!["第一段".to_string(), "第二段".to_string()],
                &expected_raw
            )
        );
        test.codex.flush_rollout().await?;
        let rollout = std::fs::read_to_string(test.codex.rollout_path().expect("rollout path"))?;
        let items = rollout
            .lines()
            .map(codex_rollout::parse_rollout_line)
            .collect::<Result<Vec<_>, _>>()?;
        let history = test
            .thread_store
            .load_presentation_history(LoadThreadHistoryParams {
                thread_id: test.session_configured.thread_id,
                include_archived: false,
            })
            .await?;
        let rollout_items = items.into_iter().map(|line| line.item).collect::<Vec<_>>();
        for items in [rollout_items, history.items] {
            for expected in &completions {
                let completed = items
                    .iter()
                    .filter_map(|entry| match entry {
                        RolloutItem::EventMsg(EventMsg::ItemCompleted(event))
                            if event.item.id() == expected.id() =>
                        {
                            Some(&event.item)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(completed.len(), 1);
                assert_eq!(
                    serde_json::to_value(completed[0])?,
                    serde_json::to_value(expected)?
                );
            }
            if !matches!(
                ending,
                Ending::Complete
                    | Ending::CompleteWithoutRawContent
                    | Ending::CompleteEmptyRawContent
            ) {
                assert!(!items.iter().any(|entry| matches!(entry,
                RolloutItem::ResponseItem(envelope) if envelope.item.id().is_some_and(|id| id.as_str() == "reasoning-partial"))));
            }
        }
        test.codex
            .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
                text: "followup".into(),
                text_elements: vec![],
            }]))
            .await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
        let requests = server.requests().await;
        assert_eq!(requests.len(), 2);
        let request: serde_json::Value = serde_json::from_slice(&requests[1])?;
        let input = request["input"].as_array().expect("request input");
        let reasoning = input
            .iter()
            .filter(|item| item["type"] == "reasoning")
            .collect::<Vec<_>>();
        let last_messages = input
            .iter()
            .filter(|item| {
                item["role"] == "assistant"
                    && item["content"]
                        .as_array()
                        .is_some_and(|parts| parts.iter().any(|part| part["text"] == "last 中文🙂"))
            })
            .count();
        if matches!(
            ending,
            Ending::Complete | Ending::CompleteWithoutRawContent | Ending::CompleteEmptyRawContent
        ) {
            assert_eq!(reasoning.len(), 1);
            assert!(
                reasoning[0]["encrypted_content"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
            );
            assert_eq!(last_messages, 1);
        } else {
            assert!(
                reasoning.is_empty(),
                "synthetic partial reasoning must not reach the model"
            );
            assert_eq!(last_messages, 0);
        }
        assert_eq!(
            input
                .iter()
                .filter(|item| item["role"] == "assistant"
                    && item["content"].as_array().is_some_and(|parts| parts
                        .iter()
                        .any(|part| part["text"] == "first 中文🙂")))
                .count(),
            1
        );
        server.shutdown().await;
        Ok(())
    })
}

/// The turn future is deep enough to overflow Windows' 1 MiB default test
/// thread stack in debug builds; run it on its own larger stack, keeping the
/// current-thread runtime that `#[tokio::test]` used.
fn run_full_text_terminal(
    name: &str,
    ending: Ending,
    history_mode: ThreadHistoryMode,
) -> anyhow::Result<()> {
    const TEST_STACK_SIZE_BYTES: usize = 8 * 1024 * 1024;
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(TEST_STACK_SIZE_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(full_text_terminal(ending, history_mode))
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("{name} thread panicked"))?
}

#[test]
fn full_text_terminal_cancel() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_cancel",
        Ending::Cancel,
        ThreadHistoryMode::Paginated,
    )
}

#[test]
fn full_text_terminal_error() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_error",
        Ending::Error,
        ThreadHistoryMode::Paginated,
    )
}

#[test]
fn full_text_terminal_eof() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_eof",
        Ending::Eof,
        ThreadHistoryMode::Paginated,
    )
}

#[test]
fn full_text_terminal_normal() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_normal",
        Ending::Complete,
        ThreadHistoryMode::Paginated,
    )
}

#[test]
fn full_text_terminal_legacy_eof() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_legacy_eof",
        Ending::Eof,
        ThreadHistoryMode::Legacy,
    )
}

#[test]
fn full_text_terminal_legacy_cancel() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_legacy_cancel",
        Ending::Cancel,
        ThreadHistoryMode::Legacy,
    )
}

#[test]
fn full_text_terminal_legacy_error() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_legacy_error",
        Ending::Error,
        ThreadHistoryMode::Legacy,
    )
}

#[test]
fn full_text_terminal_normal_done_without_raw_content() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_normal_done_without_raw_content",
        Ending::CompleteWithoutRawContent,
        ThreadHistoryMode::Paginated,
    )
}

#[test]
fn full_text_terminal_normal_done_explicit_empty_content() -> anyhow::Result<()> {
    run_full_text_terminal(
        "full_text_terminal_normal_done_explicit_empty_content",
        Ending::CompleteEmptyRawContent,
        ThreadHistoryMode::Paginated,
    )
}
