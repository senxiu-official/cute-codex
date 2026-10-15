use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn inbound_presentation_live_replay_identity_and_plain_body() {
    let (mut chat, mut rx, _ops) = make_chatwidget_manual(None).await;
    chat.thread_id = Some(ThreadId::new());
    let _ = drain_insert_history(&mut rx);
    let item=AppServerThreadItem::FunctionCallOutput { external_input_view: None,id:"jsc_".to_owned()+&"a".repeat(64),name:"external_event".into(),namespace:Some("external".into()),output:codex_protocol::models::FunctionCallOutputBody::Text(json!({"source":{"kind":"agent","id":"worker\u{1b}[31m"},"type":"message","text":"First line.\n第二行 with control\u{1b}[31m."}).to_string())};
    let notification = codex_app_server_protocol::ServerNotification::ItemCompleted(
        codex_app_server_protocol::ItemCompletedNotification {
            thread_id: chat.thread_id.unwrap().to_string(),
            turn_id: "turn".into(),
            item: item.clone(),
            completed_at_ms: 1,
        },
    );
    chat.handle_server_notification(notification.clone(), None);
    let mut cells = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let AppEvent::InsertHistoryCell(cell) = event {
            cells.push(cell);
        }
    }
    assert_eq!(cells.len(), 1);
    let display = lines_to_single_string(&cells[0].display_lines(26));
    assert!(lines_to_single_string(&cells[0].raw_lines()).contains("第二行"));
    assert!(!display.contains('\u{1b}'));
    assert!(display.contains("External input"));
    insta::assert_snapshot!(display);
    chat.handle_server_notification(notification, None);
    assert!(drain_insert_history(&mut rx).is_empty());
    let transcript = crate::thread_transcript::thread_items_to_transcript_cells(
        chat.thread_id,
        &chat.config.cwd,
        [item.clone(), item.clone()],
        crate::thread_transcript::RawReasoningVisibility::Hidden,
        None,
    );
    assert_eq!(transcript.len(), 1);
    assert_eq!(transcript[0].display_lines(26), cells[0].display_lines(26));
    chat.replay_thread_item(item.clone(), "turn".into(), ReplayKind::ThreadSnapshot);
    assert!(drain_insert_history(&mut rx).is_empty());
    let mut distinct = item.clone();
    if let AppServerThreadItem::FunctionCallOutput { id, .. } = &mut distinct {
        *id = "other-id".into();
    }
    chat.replay_thread_item(distinct, "turn".into(), ReplayKind::ThreadSnapshot);
    assert_eq!(drain_insert_history(&mut rx).len(), 1);
    let mut malformed = item;
    if let AppServerThreadItem::FunctionCallOutput { id, output, .. } = &mut malformed {
        *id = "bad".into();
        *output = codex_protocol::models::FunctionCallOutputBody::Text(
            "{\"source\":{\"kind\":\"system\"},\"text\":\"bad\"}".into(),
        );
    }
    chat.replay_thread_item(malformed, "turn".into(), ReplayKind::ThreadSnapshot);
    assert!(drain_insert_history(&mut rx).is_empty());
}
