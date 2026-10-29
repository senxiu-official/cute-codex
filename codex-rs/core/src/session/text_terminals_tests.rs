use super::*;
use codex_protocol::items::AgentMessageItem;
use codex_protocol::items::ReasoningItem;
use codex_protocol::protocol::AgentMessageContentDeltaEvent;
use codex_protocol::protocol::AgentReasoningSectionBreakEvent;
use codex_protocol::protocol::ReasoningContentDeltaEvent;
use codex_protocol::protocol::ReasoningRawContentDeltaEvent;
use codex_protocol::protocol::TurnAbortReason;
use codex_protocol::protocol::TurnAbortedEvent;
use pretty_assertions::assert_eq;

#[path = "text_terminal_abort_tests.rs"]
mod abort_tests;

fn message(id: &str) -> TurnItem {
    TurnItem::AgentMessage(AgentMessageItem {
        id: id.into(),
        content: vec![],
        phase: None,
        memory_citation: None,
        delivery: None,
        questions: None,
    })
}

#[tokio::test]
async fn forced_abort_terminal_closes_interleaved_public_text_once() {
    let (session, turn, rx) = crate::session::tests::make_session_and_context_with_rx().await;
    let items = vec![
        message("msg-a"),
        TurnItem::Reasoning(ReasoningItem {
            id: "reasoning-a".into(),
            summary_text: vec![],
            raw_content: vec![],
        }),
        message("msg-b"),
    ];
    for item in &items {
        session.emit_turn_item_started(&turn, item).await;
    }
    for (id, delta) in [("msg-a", "中文"), ("msg-b", "other"), ("msg-a", "🙂")] {
        session
            .send_event(
                &turn,
                EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent {
                    thread_id: session.thread_id.to_string(),
                    turn_id: turn.sub_id.clone(),
                    item_id: id.into(),
                    delta: delta.into(),
                }),
            )
            .await;
    }
    for (index, delta) in [(1, "second"), (0, "first"), (1, " part")] {
        session
            .send_event(
                &turn,
                EventMsg::ReasoningContentDelta(ReasoningContentDeltaEvent {
                    thread_id: session.thread_id.to_string(),
                    turn_id: turn.sub_id.clone(),
                    item_id: "reasoning-a".into(),
                    delta: delta.into(),
                    summary_index: index,
                }),
            )
            .await;
        session
            .send_event(
                &turn,
                EventMsg::ReasoningRawContentDelta(ReasoningRawContentDeltaEvent {
                    thread_id: session.thread_id.to_string(),
                    turn_id: turn.sub_id.clone(),
                    item_id: "reasoning-a".into(),
                    delta: delta.into(),
                    content_index: index,
                }),
            )
            .await;
    }
    session
        .send_event(
            &turn,
            EventMsg::AgentReasoningSectionBreak(AgentReasoningSectionBreakEvent {
                item_id: "reasoning-a".into(),
                summary_index: 2,
            }),
        )
        .await;
    session
        .send_event(
            &turn,
            EventMsg::TurnAborted(TurnAbortedEvent {
                root_turn_id: None,
                turn_id: Some(turn.sub_id.clone()),
                reason: TurnAbortReason::Interrupted,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
            }),
        )
        .await;
    let mut completed = vec![];
    while let Ok(event) = rx.try_recv() {
        match event.msg {
            EventMsg::ItemCompleted(event) => {
                completed.push(serde_json::to_value(event.item).unwrap())
            }
            EventMsg::TurnAborted(_) => assert_eq!(completed.len(), 3),
            _ => {}
        }
    }
    let mut expected = items;
    if let TurnItem::AgentMessage(item) = &mut expected[0] {
        item.content = vec![AgentMessageContent::Text {
            text: "中文🙂".into(),
        }];
    }
    if let TurnItem::Reasoning(item) = &mut expected[1] {
        item.summary_text = vec!["first".into(), "second part".into(), "".into()];
        item.raw_content = vec!["first".into(), "second part".into()];
    }
    if let TurnItem::AgentMessage(item) = &mut expected[2] {
        item.content = vec![AgentMessageContent::Text {
            text: "other".into(),
        }];
    }
    assert_eq!(
        completed,
        expected
            .into_iter()
            .map(|item| serde_json::to_value(item).unwrap())
            .collect::<Vec<_>>()
    );
    let prompt = session.clone_history().await.for_prompt(&[]);
    assert!(
        !prompt
            .iter()
            .any(|item| item.id().is_some_and(|id| id.as_str() == "reasoning-a"))
    );
    session.complete_open_text_items(&turn).await;
    assert!(rx.try_recv().is_err());
}
