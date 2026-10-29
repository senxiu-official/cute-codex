use super::thread_history::build_turns_from_rollout_items;
use super::v2::ThreadItem;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::AgentMessageItem;
use codex_protocol::items::ReasoningItem;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::TurnStartedEvent;
use codex_rollout::RolloutItem;
use pretty_assertions::assert_eq;

#[test]
fn persisted_full_text_terminals_replace_flattened_echoes_without_losing_parts() {
    let text = vec![
        TurnItem::AgentMessage(AgentMessageItem {
            id: "message".into(),
            content: vec![AgentMessageContent::Text {
                text: "中文🙂".into(),
            }],
            phase: None,
            memory_citation: None,
            delivery: None,
            questions: None,
        }),
        TurnItem::Reasoning(ReasoningItem {
            id: "reasoning".into(),
            summary_text: vec!["first".into(), "".into(), "last".into()],
            raw_content: vec!["raw".into(), "".into()],
        }),
    ];
    for raw_visible in [true, false] {
        let mut items = vec![RolloutItem::EventMsg(EventMsg::TurnStarted(
            TurnStartedEvent {
                turn_id: "turn".into(),
                root_turn_id: None,
                trace_id: None,
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
                turn_attribution: None,
            },
        ))];
        for item in &text {
            items.push(RolloutItem::EventMsg(EventMsg::ItemCompleted(
                ItemCompletedEvent {
                    thread_id: ThreadId::default(),
                    turn_id: "turn".into(),
                    item: item.clone(),
                    started_at_ms: None,
                    completed_at_ms: 1,
                },
            )));
            items.extend(
                item.as_legacy_events(raw_visible)
                    .into_iter()
                    .map(RolloutItem::EventMsg),
            );
        }
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(
            turns[0].items,
            text.iter()
                .cloned()
                .map(ThreadItem::from)
                .collect::<Vec<_>>()
        );
    }
}
