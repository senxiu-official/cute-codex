//! Derived display of complete authoritative pairs; never a new durable event.
use crate::protocol::thread_history::ThreadHistoryItemChange;
use crate::protocol::v2::ThreadItem;
use codex_protocol::external_input::Commit;
use codex_protocol::external_input::Error;
use codex_protocol::external_input::Receipt;
use codex_protocol::external_input_record::Fact;
use codex_rollout::RolloutItem;

/// Keeps only the immediately preceding commit while reading an ordered rollout.
#[derive(Default)]
pub struct ExternalInputProjection {
    pending: Option<Commit>,
    seen: std::collections::BTreeMap<String, (String, String)>,
}
impl ExternalInputProjection {
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn observe(
        &mut self,
        item: &RolloutItem,
    ) -> Result<Option<ThreadHistoryItemChange>, Error> {
        if let Some(commit) = self.pending.take() {
            let RolloutItem::ResponseItem(actual) = item else {
                return Err(Error::Corrupt);
            };
            let envelope = &commit.envelope;
            envelope.validate()?;
            if commit.receipt
                != Receipt::new(
                    envelope,
                    commit.receipt.turn_id.clone(),
                    commit.receipt.ordinal,
                )?
                || actual.item != envelope.response_item()?
            {
                return Err(Error::Corrupt);
            }
            let identity = (
                envelope.semantic_sha256.clone(),
                commit.receipt.receipt_id.clone(),
            );
            if self
                .seen
                .get(&envelope.message.id)
                .is_some_and(|old| old != &identity)
            {
                return Err(Error::Conflict);
            }
            self.seen.insert(envelope.message.id.clone(), identity);
            let codex_protocol::models::ResponseItem::FunctionCallOutput {
                id: Some(id),
                name: Some(name),
                namespace,
                output,
                ..
            } = &actual.item
            else {
                return Err(Error::Corrupt);
            };
            return Ok(Some(ThreadHistoryItemChange {
                turn_id: commit.receipt.turn_id,
                item: ThreadItem::FunctionCallOutput {
                    id: id.to_string(),
                    name: name.clone(),
                    namespace: namespace.clone(),
                    output: output.body.clone(),
                    external_input_view: envelope.view.clone(),
                },
                started_at_ms: None,
                completed_at_ms: None,
            }));
        }
        // A standalone generic FCO is not evidence of external admission. Leave it to
        // its existing projection; never synthesize an external display from it.
        if let RolloutItem::ExternalInput(record) = item {
            if record.version != 1 {
                return Err(Error::Version);
            }
            if let Fact::Commit { commit } = &record.fact {
                if record.owner_id != commit.envelope.owner_id
                    || record.thread_id != commit.envelope.thread_id
                {
                    return Err(Error::Conflict);
                }
                commit.envelope.validate()?;
                self.pending = Some((**commit).clone());
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
#[path = "external_input_projection_tests.rs"]
mod tests;
