//! Read-only legacy timeline reconstruction; never converts the source rollout.
use super::super::LocalThreadStore;
use super::realtime::TimelineCursor;
use super::realtime::entry_key;
use super::thread_history_error;
use crate::ListTimelineParams;
use crate::ThreadStoreResult;
use crate::TimelinePage;
use codex_app_server_protocol::ThreadHistoryBuilder;
use codex_app_server_protocol::ThreadTimelineEntry;
use codex_app_server_protocol::TurnStatus;
use codex_protocol::protocol::EventMsg;
use codex_rollout::RolloutItem;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

#[derive(PartialEq, Eq)]
struct SourceStamp {
    thread_id: codex_protocol::ThreadId,
    path: PathBuf,
    len: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

pub(in crate::local) struct CachedTimeline {
    stamp: SourceStamp,
    entries: Arc<Vec<ThreadTimelineEntry>>,
}

impl SourceStamp {
    async fn read(thread_id: codex_protocol::ThreadId, path: &Path) -> ThreadStoreResult<Self> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(thread_history_error)?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            thread_id,
            path: path.to_path_buf(),
            len: metadata.len(),
            modified: metadata.modified().map_err(thread_history_error)?,
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

pub(super) async fn list(
    store: &LocalThreadStore,
    params: ListTimelineParams,
) -> ThreadStoreResult<TimelinePage> {
    let source = super::super::thread_rollout_resolver::resolve_current(store, params.thread_id)
        .await?
        .ok_or(crate::ThreadStoreError::ThreadNotFound {
            thread_id: params.thread_id,
        })?;
    let entries = cached_entries(store, params.thread_id, &source.path).await?;
    let mut start = 0;
    if let Some(cursor) = params.cursor {
        let cursor: TimelineCursor = serde_json::from_str(&cursor).map_err(thread_history_error)?;
        if cursor.thread_id != params.thread_id || cursor.kind > 4 {
            return Err(thread_history_error("invalid legacy timeline cursor"));
        }
        start = entries.partition_point(|entry| {
            entry_key(entry) >= (cursor.position, cursor.kind, cursor.id.as_str())
        });
    }
    let end = start.saturating_add(params.page_size).min(entries.len());
    let more = end < entries.len();
    let mut entries = entries[start..end].to_vec();
    let next_cursor = if more {
        entries
            .last()
            .map(|entry| {
                let (position, kind, id) = entry_key(entry);
                serde_json::to_string(&TimelineCursor {
                    thread_id: params.thread_id,
                    position,
                    kind,
                    id: id.to_owned(),
                })
                .map_err(thread_history_error)
            })
            .transpose()?
    } else {
        None
    };
    entries.reverse();
    Ok(TimelinePage {
        items: entries,
        next_cursor,
        active_realtime_session_at_page_start: None,
    })
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "serialize rollout cache misses across file reads; reconstruction never reenters this cache"
)]
async fn cached_entries(
    store: &LocalThreadStore,
    thread_id: codex_protocol::ThreadId,
    path: &Path,
) -> ThreadStoreResult<Arc<Vec<ThreadTimelineEntry>>> {
    // One slot per store bounds retained histories. Serialize misses so simultaneous
    // readers do not each rebuild the same long rollout.
    let mut cache = store.legacy_timeline.lock().await;
    let stamp = SourceStamp::read(thread_id, path).await?;
    if let Some(cached) = cache.as_ref()
        && cached.stamp == stamp
    {
        return Ok(Arc::clone(&cached.entries));
    }
    *cache = None;
    let entries = Arc::new(reconstruct(path).await?);
    // Do not retain a reconstruction if a writer appended/replaced its source
    // during the read. Exceptionally large sources remain readable, uncached.
    if stamp.len <= 256 * 1024 * 1024 && SourceStamp::read(thread_id, path).await? == stamp {
        *cache = Some(CachedTimeline {
            stamp,
            entries: Arc::clone(&entries),
        });
    }
    Ok(entries)
}

async fn reconstruct(path: &Path) -> ThreadStoreResult<Vec<ThreadTimelineEntry>> {
    // This reader rejects undecodable rows before deriving positions or absence.
    let items = super::super::read_thread::load_presentation_history_items(path).await?;
    let mut builder = ThreadHistoryBuilder::new();
    let mut ordinary = BTreeMap::new();
    let mut starts = BTreeMap::new();
    let mut ends = BTreeMap::new();
    let mut displays = BTreeMap::new();
    for (position, item) in items.iter().enumerate() {
        let position = position as u64;
        if let RolloutItem::EventMsg(EventMsg::PresentationAppended(record)) = item {
            record.validate().map_err(thread_history_error)?;
            displays
                .entry(record.presentation.id.clone())
                .or_insert_with(|| ThreadTimelineEntry::Presentation {
                    position,
                    item: record.clone(),
                });
        }
        let changes = builder.handle_rollout_item_with_changes(item);
        for removed in changes.removed_turn_ids {
            starts.remove(&removed);
            ends.remove(&removed);
            ordinary.retain(|(turn, _), _| turn != &removed);
        }
        for changed in changes.changed_items {
            let key = (changed.turn_id.clone(), changed.item.id().to_string());
            let entry = ordinary
                .entry(key)
                .or_insert_with(|| ThreadTimelineEntry::Item {
                    position,
                    turn_id: changed.turn_id,
                    item: Box::new(changed.item.clone()),
                });
            if let ThreadTimelineEntry::Item { item, .. } = entry {
                **item = changed.item;
            }
        }
        for turn in changes.changed_turns {
            starts.entry(turn.turn_id.clone()).or_insert_with(|| {
                ThreadTimelineEntry::TurnStarted {
                    position,
                    turn_id: turn.turn_id.clone(),
                    started_at: turn.started_at,
                }
            });
            if turn.status != TurnStatus::InProgress {
                ends.insert(
                    turn.turn_id.clone(),
                    ThreadTimelineEntry::TurnCompleted {
                        position,
                        turn_id: turn.turn_id,
                        status: turn.status,
                        error: turn.error,
                        started_at: turn.started_at,
                        completed_at: turn.completed_at,
                        duration_ms: turn.duration_ms,
                    },
                );
            }
        }
    }
    builder.finish_checked().map_err(thread_history_error)?;
    let mut entries = ordinary
        .into_values()
        .chain(starts.into_values())
        .chain(ends.into_values())
        .chain(displays.into_values())
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| entry_key(b).cmp(&entry_key(a)));
    Ok(entries)
}

#[cfg(test)]
#[path = "legacy_timeline_tests.rs"]
mod tests;
