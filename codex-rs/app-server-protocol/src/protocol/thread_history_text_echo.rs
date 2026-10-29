use codex_protocol::items::TurnItem;
use codex_protocol::protocol::EventMsg;
use std::collections::VecDeque;

/// Suppress only the flattened legacy echoes of a persisted full text terminal.
/// Keeping those echoes on disk lets older runtimes still read legacy history.
#[derive(Default)]
pub(super) struct TextEcho(VecDeque<EventMsg>);

impl TextEcho {
    pub(super) fn observe(&mut self, event: &EventMsg) -> bool {
        if let EventMsg::ItemCompleted(completed) = event
            && matches!(
                completed.item,
                TurnItem::AgentMessage(_) | TurnItem::Reasoning(_)
            )
        {
            self.0 = completed
                .item
                .as_legacy_events(/*show_raw_agent_reasoning*/ true)
                .into();
            return false;
        }
        let echo = match (self.0.front(), event) {
            (Some(EventMsg::AgentMessage(expected)), EventMsg::AgentMessage(actual)) => {
                expected.message == actual.message
            }
            (Some(EventMsg::AgentReasoning(expected)), EventMsg::AgentReasoning(actual)) => {
                expected.text == actual.text
            }
            (
                Some(EventMsg::AgentReasoningRawContent(expected)),
                EventMsg::AgentReasoningRawContent(actual),
            ) => expected.text == actual.text,
            _ => false,
        };
        if echo {
            self.0.pop_front();
        } else if matches!(
            event,
            EventMsg::AgentMessage(_)
                | EventMsg::AgentReasoning(_)
                | EventMsg::AgentReasoningRawContent(_)
                | EventMsg::TurnStarted(_)
                | EventMsg::TurnComplete(_)
                | EventMsg::TurnAborted(_)
        ) {
            self.0.clear();
        }
        echo
    }
}
