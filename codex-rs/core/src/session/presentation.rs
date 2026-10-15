//! Owner-bound display persistence. No scheduling or context mutation.
use crate::CodexThread;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::ensure;
use codex_history::RolloutItem;
use codex_protocol::presentation::PresentationAppended;
use codex_protocol::presentation::PresentationReferenceKind;
use codex_protocol::protocol::EventMsg;
use std::collections::BTreeMap;

impl CodexThread {
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "receipt recovery must exclude concurrent presentation admission"
    )]
    pub async fn presentation_status(
        &self,
        owner: &str,
        id: &str,
        digest: &str,
    ) -> Result<Option<PresentationAppended>> {
        let _serial = self.session.presentation.lock().await;
        self.require_presentation_owner(owner).await?;
        let found = self.presentation_history().await?.get(id).cloned();
        if let Some(record) = &found {
            ensure!(
                record.semantic_sha256 == digest,
                "presentation identity conflict"
            );
        }
        Ok(found)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "identity check and fallible durable append must be serialized together"
    )]
    pub async fn append_presentation(
        &self,
        record: PresentationAppended,
    ) -> Result<(PresentationAppended, bool)> {
        record.validate().map_err(|error| anyhow!(error))?;
        ensure!(
            record.origin_thread_id == self.session.thread_id.to_string(),
            "presentation thread mismatch"
        );
        let _serial = self.session.presentation.lock().await;
        self.require_presentation_owner(&record.owner_id).await?;
        if let Some(previous) = self
            .presentation_history()
            .await?
            .get(&record.presentation.id)
        {
            ensure!(
                previous == &record,
                "presentation identity conflict or inherited identity"
            );
            return Ok((previous.clone(), false));
        }
        let live = self
            .session
            .services
            .live_thread
            .as_ref()
            .ok_or_else(|| anyhow!("presentation requires persistent history"))?;
        let metadata = live.read_thread(false, false).await?;
        let history = live.load_presentation_history().await?;
        if !record.presentation.references.is_empty() {
            ensure!(
                metadata.forked_from_id.is_none(),
                "presentation references on inherited history are unsupported"
            );
        }
        for reference in &record.presentation.references {
            let known = history.items.iter().any(|item| match (&reference.kind, item) {
                (PresentationReferenceKind::ExternalInput, RolloutItem::ExternalInput(entry)) => {
                    matches!(&entry.fact, codex_protocol::external_input_record::Fact::Commit { commit }
                        if commit.envelope.thread_id == record.origin_thread_id && commit.envelope.message.id == reference.id)
                }
                (PresentationReferenceKind::McpInvocation, RolloutItem::EventMsg(EventMsg::ItemCompleted(event))) => {
                    matches!(&event.item, codex_protocol::items::TurnItem::McpToolCall(item) if item.id == reference.id)
                }
                (PresentationReferenceKind::McpInvocation, RolloutItem::EventMsg(EventMsg::McpToolCallEnd(event))) => event.call_id == reference.id,
                _ => false,
            });
            ensure!(
                known,
                "presentation reference is not a known same-thread invocation/input"
            );
        }
        live.append_items(&[RolloutItem::EventMsg(EventMsg::PresentationAppended(
            record.clone(),
        ))])
        .await?;
        live.flush().await?;
        live.persist_for_read().await?;
        ensure!(
            self.presentation_history()
                .await?
                .get(&record.presentation.id)
                == Some(&record),
            "presentation persistence outcome unknown"
        );
        Ok((record, true))
    }

    /// Current authoritative timeline position; not part of receipt identity.
    pub async fn presentation_position(&self, id: &str) -> Result<u64> {
        let live = self
            .session
            .services
            .live_thread
            .as_ref()
            .ok_or_else(|| anyhow!("presentation requires persistent history"))?;
        let path = live
            .local_rollout_path()
            .await?
            .ok_or_else(|| anyhow!("presentation requires local history"))?;
        let meta = codex_rollout::read_session_meta_line(&path).await?;
        if meta.meta.history_mode == codex_protocol::protocol::ThreadHistoryMode::Legacy {
            let (items, _, errors) =
                codex_rollout::RolloutRecorder::load_rollout_items(&path).await?;
            ensure!(errors == 0, "unreadable presentation position source");
            return items.iter().position(|item| matches!(item, RolloutItem::EventMsg(EventMsg::PresentationAppended(record)) if record.presentation.id == id))
                .map(|position| position as u64).ok_or_else(|| anyhow!("presentation position unavailable"));
        }
        let mut reader = codex_rollout::open_rollout_line_reader(&path).await?;
        while let Some(line) = reader.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(&line)?;
            if codex_rollout::is_retired_rollout_line(&value) {
                continue;
            }
            let line = codex_rollout::decode_rollout_line(value)?;
            if let RolloutItem::EventMsg(EventMsg::PresentationAppended(record)) = line.item {
                record.validate().map_err(|error| anyhow!(error))?;
                if record.presentation.id == id {
                    return line
                        .ordinal
                        .ok_or_else(|| anyhow!("missing presentation position"));
                }
            }
        }
        Err(anyhow!("presentation position unavailable"))
    }

    async fn require_presentation_owner(&self, owner: &str) -> Result<()> {
        let state = self.session.external_input.lock().await;
        let bound = state
            .as_ref()
            .ok_or_else(|| anyhow!("presentation capability disabled"))?;
        ensure!(
            bound.owner == owner && !bound.poisoned,
            "presentation owner unavailable"
        );
        Ok(())
    }

    async fn presentation_history(&self) -> Result<BTreeMap<String, PresentationAppended>> {
        let live = self
            .session
            .services
            .live_thread
            .as_ref()
            .ok_or_else(|| anyhow!("presentation requires persistent history"))?;
        // Resolve queued writes before deciding absence, without resubmission.
        live.persist_for_read().await?;
        let history = live.load_presentation_history().await?;
        let mut records = BTreeMap::new();
        for item in history.items {
            if let RolloutItem::EventMsg(EventMsg::PresentationAppended(record)) = item {
                record.validate().map_err(|error| anyhow!(error))?;
                if let Some(previous) =
                    records.insert(record.presentation.id.clone(), record.clone())
                {
                    ensure!(previous == record, "conflicting presentation history");
                }
            }
        }
        Ok(records)
    }
}
