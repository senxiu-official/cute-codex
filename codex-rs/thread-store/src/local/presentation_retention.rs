//! Preserve display facts removed from the model prefix by paginated revert.
use super::rollout_lineage::RolloutLineage;
use crate::ThreadStoreError;
use crate::ThreadStoreResult;
use codex_protocol::protocol::EventMsg;
use codex_rollout::RolloutItem;

pub(super) async fn suffix(
    lineage: &RolloutLineage,
    cut: u64,
) -> ThreadStoreResult<Vec<RolloutItem>> {
    let mut retained = Vec::new();
    for segment in lineage.segments() {
        let mut reader = codex_rollout::open_rollout_line_reader(&segment.rollout_path)
            .await
            .map_err(error)?;
        while let Some(raw) = reader.next_line().await.map_err(error)? {
            if raw.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(&raw).map_err(error)?;
            if codex_rollout::is_retired_rollout_line(&value) {
                continue;
            }
            let line = codex_rollout::decode_rollout_line(value).map_err(error)?;
            let ordinal = line
                .ordinal
                .ok_or_else(|| error("missing presentation source ordinal"))?;
            if ordinal < cut
                || ordinal < segment.start_ordinal()
                || segment.end_ordinal().is_some_and(|end| ordinal >= end)
            {
                continue;
            }
            if let RolloutItem::EventMsg(EventMsg::PresentationAppended(record)) = &line.item {
                record.validate().map_err(error)?;
                retained.push(line.item);
            }
        }
    }
    Ok(retained)
}
fn error(error: impl std::fmt::Display) -> ThreadStoreError {
    ThreadStoreError::Internal {
        message: format!("presentation retention: {error}"),
    }
}
