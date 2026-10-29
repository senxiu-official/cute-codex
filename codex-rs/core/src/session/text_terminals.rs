//! Retain public streamed text until its authoritative item terminal is delivered.

use super::Session;
use super::TurnContext;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ItemCompletedEvent;

#[derive(Debug, Default)]
pub(crate) struct OpenTextItems {
    items: Vec<TurnItem>,
    started: std::collections::HashSet<String>,
}

impl OpenTextItems {
    pub(crate) fn observe(&mut self, event: &EventMsg) {
        match event {
            EventMsg::ItemStarted(event)
                if matches!(
                    event.item,
                    TurnItem::AgentMessage(_) | TurnItem::Reasoning(_)
                ) =>
            {
                self.started.insert(event.item.id());
                if !self.items.iter().any(|item| item.id() == event.item.id()) {
                    self.items.push(event.item.clone());
                }
            }
            EventMsg::AgentMessageContentDelta(event) => {
                if let Some(TurnItem::AgentMessage(item)) = self.find(&event.item_id) {
                    if item.content.is_empty() {
                        item.content.push(AgentMessageContent::Text {
                            text: String::new(),
                        });
                    }
                    let Some(AgentMessageContent::Text { text }) = item.content.last_mut() else {
                        return;
                    };
                    text.push_str(&event.delta);
                }
            }
            EventMsg::ReasoningContentDelta(event) => {
                if let Some(TurnItem::Reasoning(item)) = self.find(&event.item_id) {
                    append_part(&mut item.summary_text, event.summary_index, &event.delta);
                }
            }
            EventMsg::ReasoningRawContentDelta(event) => {
                if let Some(TurnItem::Reasoning(item)) = self.find(&event.item_id) {
                    append_part(&mut item.raw_content, event.content_index, &event.delta);
                }
            }
            EventMsg::AgentReasoningSectionBreak(event) => {
                if let Some(TurnItem::Reasoning(item)) = self.find(&event.item_id) {
                    append_part(&mut item.summary_text, event.summary_index, "");
                }
            }
            _ => {}
        }
    }

    pub(crate) fn stage_completed(&mut self, response: &ResponseItem, item: &mut TurnItem) {
        // A provider may omit public raw reasoning in its canonical done item.
        // Preserve already emitted content for an absent field; explicit empty
        // content remains authoritative and clears the streamed parts.
        if let (ResponseItem::Reasoning { content: None, .. }, TurnItem::Reasoning(finalized)) =
            (response, &mut *item)
            && let Some(TurnItem::Reasoning(streamed)) = self.find(&finalized.id)
        {
            finalized.raw_content.clone_from(&streamed.raw_content);
        }
        self.stage(item);
    }

    pub(crate) fn stage(&mut self, item: &TurnItem) {
        if !matches!(item, TurnItem::AgentMessage(_) | TurnItem::Reasoning(_)) {
            return;
        }
        if let Some(previous) = self.find(&item.id()) {
            *previous = item.clone();
        } else {
            self.items.push(item.clone());
        }
    }

    fn find(&mut self, id: &str) -> Option<&mut TurnItem> {
        self.items.iter_mut().find(|item| item.id() == id)
    }

    pub(crate) fn complete(&mut self, id: &str) {
        self.items.retain(|item| item.id() != id);
        self.started.remove(id);
    }
}

fn append_part(parts: &mut Vec<String>, index: i64, delta: &str) {
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    let Some(len) = index.checked_add(1) else {
        return;
    };
    if len > parts.len() {
        // Refuse an invalid/unallocatable upstream index without panicking.
        if parts.try_reserve(len - parts.len()).is_err() {
            return;
        }
        parts.resize_with(len, String::new);
    }
    parts[index].push_str(delta);
}

impl Session {
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "repair must commit each full terminal and flush before a worker can be aborted"
    )]
    pub(crate) fn complete_open_text_items<'a>(
        &'a self,
        turn: &'a TurnContext,
    ) -> futures::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            let _commit = turn.text_terminal_commit.lock().await;
            let items = turn.open_text_items.lock().await.items.clone();
            if items.is_empty() {
                return;
            }
            for item in items {
                let started = turn
                    .open_text_items
                    .lock()
                    .await
                    .started
                    .contains(&item.id());
                if !started {
                    self.emit_turn_item_started(turn, &item).await;
                }
                let completed_at_ms = super::now_unix_timestamp_ms();
                let started_at_ms = turn.turn_timing_state.take_item_started(&item.id()).await;
                // This is display-only public output. In particular, partial
                // reasoning has no encrypted model continuation and must never
                // become a synthetic ResponseItem in a later model request.
                self.send_event_inner(
                    turn,
                    EventMsg::ItemCompleted(ItemCompletedEvent {
                        thread_id: self.thread_id,
                        turn_id: turn.sub_id.clone(),
                        item,
                        started_at_ms,
                        completed_at_ms,
                    }),
                )
                .await;
            }
            // A client may immediately read history after completion/TurnAborted.
            if let Err(error) = self.flush_rollout().await {
                tracing::warn!(%error, "failed to flush partial text item terminals");
            }
        })
    }
}

#[cfg(test)]
#[path = "text_terminals_tests.rs"]
mod tests;
