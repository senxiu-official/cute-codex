use super::*;
use codex_protocol::ThreadId;
use codex_protocol::protocol::SessionMeta;
use codex_protocol::protocol::SessionMetaLine;
use codex_protocol::protocol::TurnCompleteEvent;
use codex_protocol::protocol::TurnStartedEvent;
use codex_rollout::RolloutLine;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

fn history(thread_id: ThreadId, names: &[&str]) -> String {
    let mut items = vec![RolloutItem::SessionMeta(SessionMetaLine {
        meta: SessionMeta {
            session_id: thread_id.into(),
            id: thread_id,
            ..Default::default()
        },
        git: None,
    })];
    for name in names {
        items.push(RolloutItem::EventMsg(EventMsg::TurnStarted(
            TurnStartedEvent {
                turn_id: (*name).into(),
                trace_id: None,
                turn_attribution: None,
                root_turn_id: None,
                started_at: Some(10),
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            },
        )));
        items.push(RolloutItem::EventMsg(EventMsg::TurnComplete(
            TurnCompleteEvent {
                turn_id: (*name).into(),
                last_agent_message: None,
                error: None,
                started_at: Some(10),
                completed_at: Some(20),
                duration_ms: Some(10_000),
                time_to_first_token_ms: None,
                root_turn_id: None,
            },
        )));
    }
    items
        .into_iter()
        .map(|item| {
            serde_json::to_string(&RolloutLine {
                timestamp: "2026-07-16T00:00:00.000Z".into(),
                ordinal: None,
                item,
            })
            .unwrap()
                + "\n"
        })
        .collect()
}

#[tokio::test]
async fn legacy_cache_reuses_history_and_invalidates_append_replacement_and_errors() {
    let home = TempDir::new().unwrap();
    let store = LocalThreadStore::new(crate::local::test_support::test_config(home.path()), None);
    let id = ThreadId::new();
    let path = home.path().join("history.jsonl");
    std::fs::write(&path, history(id, &["turn-1"])).unwrap();
    let first = cached_entries(&store, id, &path).await.unwrap();
    let (second, concurrent) = tokio::join!(
        cached_entries(&store, id, &path),
        cached_entries(&store, id, &path)
    );
    assert!(Arc::ptr_eq(&first, &second.unwrap()));
    assert!(Arc::ptr_eq(&first, &concurrent.unwrap()));
    std::fs::write(&path, history(id, &["turn-1", "turn-2"])).unwrap();
    let appended = cached_entries(&store, id, &path).await.unwrap();
    assert!(!Arc::ptr_eq(&first, &appended));
    assert_eq!(appended.as_ref(), &reconstruct(&path).await.unwrap());
    // A same-size atomic replacement must not reuse the previous source.
    let replacement = home.path().join("replacement.jsonl");
    std::fs::write(&replacement, history(id, &["turn-3", "turn-4"])).unwrap();
    std::fs::rename(&replacement, &path).unwrap();
    let replaced = cached_entries(&store, id, &path).await.unwrap();
    assert!(!Arc::ptr_eq(&appended, &replaced));
    assert_eq!(replaced.as_ref(), &reconstruct(&path).await.unwrap());
    std::fs::write(&path, history(id, &["turn-3"])).unwrap();
    assert_eq!(
        cached_entries(&store, id, &path).await.unwrap().as_ref(),
        &reconstruct(&path).await.unwrap()
    );
    std::fs::write(&path, "malformed\n").unwrap();
    assert!(cached_entries(&store, id, &path).await.is_err());
    assert!(store.legacy_timeline.lock().await.is_none());
}

#[tokio::test]
async fn legacy_pages_share_reconstruction_and_preserve_order() {
    use codex_protocol::protocol::SessionSource;
    let home = TempDir::new().unwrap();
    let config = crate::local::test_support::test_config(home.path());
    let db = codex_state::StateRuntime::init(
        config.sqlite.clone(),
        config.default_model_provider_id.clone(),
    )
    .await
    .unwrap();
    let id = ThreadId::new();
    let path = home
        .path()
        .join(format!("rollout-2026-07-16T00-00-00-{id}.jsonl"));
    std::fs::write(&path, history(id, &["turn-1", "turn-2", "turn-3"])).unwrap();
    let meta = codex_state::ThreadMetadataBuilder::new(
        id,
        path.clone(),
        chrono::Utc::now(),
        SessionSource::Cli,
    );
    db.upsert_thread(&meta.build(config.default_model_provider_id.as_str()))
        .await
        .unwrap();
    let store = LocalThreadStore::new(config, Some(db));
    let expected = reconstruct(&path).await.unwrap();
    let mut cursor = None;
    let mut pages = Vec::new();
    let cached = cached_entries(&store, id, &path).await.unwrap();
    loop {
        let page = store
            .list_timeline(ListTimelineParams {
                thread_id: id,
                cursor,
                page_size: 2,
            })
            .await
            .unwrap();
        pages.push(page.items);
        assert!(Arc::ptr_eq(
            &cached,
            &cached_entries(&store, id, &path).await.unwrap()
        ));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    let mut expected = expected;
    expected.reverse();
    assert_eq!(
        pages.into_iter().rev().flatten().collect::<Vec<_>>(),
        expected
    );
}
