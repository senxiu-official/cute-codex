use super::*;
use codex_protocol::presentation::Presentation;
use codex_protocol::presentation::PresentationAppended;
use codex_protocol::presentation::PresentationFormat;
fn notice(id: &str) -> PresentationAppended {
    let mut record = PresentationAppended {
        version: 1,
        owner_id: "owner".into(),
        origin_thread_id: "origin".into(),
        presentation: Presentation {
            id: id.into(),
            source: codex_protocol::external_input::Source {
                kind: codex_protocol::external_input::SourceKind::Service,
                id: "build\u{1b}[31m".into(),
            },
            title: "Report 世界".into(),
            body: "**safe bold**\n第二行\u{7}".into(),
            format: PresentationFormat::Markdown,
            references: vec![],
        },
        semantic_sha256: String::new(),
        receipt_id: String::new(),
    };
    record.semantic_sha256 = record.semantic_digest();
    record.receipt_id = record.receipt_digest();
    record
}
#[tokio::test]
async fn durable_notice_live_replay_identity_and_safe_render() {
    let (mut chat, mut rx, _ops) = make_chatwidget_manual(None).await;
    let id = ThreadId::new();
    chat.thread_id = Some(id);
    let _ = drain_insert_history(&mut rx);
    let record = notice("one");
    let notification = ServerNotification::ThreadPresentationAppended(
        codex_app_server_protocol::PresentationAppendedNotification {
            thread_id: id.to_string(),
            position: 1,
            item: record.clone(),
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
    let display = cells[0]
        .display_lines(24)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!display.contains('\u{1b}'));
    assert!(!display.contains('\u{7}'));
    assert!(display.contains("Report"));
    insta::assert_snapshot!(display);
    let raw = cells[0]
        .raw_lines()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(raw.contains("**safe bold**"));
    assert!(!raw.contains('•'));
    chat.handle_server_notification(notification, None);
    chat.replay_presentation_timeline(
        vec![],
        vec![
            codex_app_server_protocol::ThreadTimelineEntry::Presentation {
                position: 77,
                item: record.clone(),
            },
        ],
        ReplayKind::ThreadSnapshot,
    );
    assert!(drain_insert_history(&mut rx).is_empty());
    chat.on_presentation(notice("two"));
    assert_eq!(drain_insert_history(&mut rx).len(), 1);
    let mut conflict = record;
    conflict.presentation.body = "changed".into();
    conflict.semantic_sha256 = conflict.semantic_digest();
    conflict.receipt_id = conflict.receipt_digest();
    chat.on_presentation(conflict);
    assert!(drain_insert_history(&mut rx).iter().any(|cell| {
        cell.iter()
            .any(|line| line.to_string().contains("Conflicting"))
    }));
}
#[tokio::test]
async fn durable_notice_idle_timeline_order_and_no_op() {
    let (mut chat, mut rx, mut ops) = make_chatwidget_manual(None).await;
    chat.thread_id = Some(ThreadId::new());
    let _ = drain_insert_history(&mut rx);
    chat.transcript.presentation_timeline = Some(vec![
        codex_app_server_protocol::ThreadTimelineEntry::Presentation {
            position: 1,
            item: notice("first"),
        },
        codex_app_server_protocol::ThreadTimelineEntry::Presentation {
            position: 2,
            item: notice("second"),
        },
    ]);
    chat.replay_thread_turns(vec![], ReplayKind::ResumeInitialMessages);
    assert_eq!(drain_insert_history(&mut rx).len(), 2);
    assert!(ops.try_recv().is_err());
    assert!(chat.turn_lifecycle.last_turn_id.is_none());
}

#[test]
fn durable_notice_plain_linked_supplement_keeps_explicit_identity() {
    let mut record = notice("linked");
    record.presentation.format = PresentationFormat::PlainText;
    record.presentation.references = vec![codex_protocol::presentation::PresentationReference {
        kind: codex_protocol::presentation::PresentationReferenceKind::McpInvocation,
        id: "call-17".into(),
    }];
    record.semantic_sha256 = record.semantic_digest();
    record.receipt_id = record.receipt_digest();
    let cell = history_cell::PresentationHistoryCell::new(record).unwrap();
    let display = cell
        .display_lines(24)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(display.contains("**safe bold**"));
    assert!(display.contains("call-17"));
    insta::assert_snapshot!(display);
}

#[tokio::test]
async fn durable_notice_adjacent_group_both_orders_and_reconnect() {
    use codex_app_server_protocol::ThreadTimelineEntry as Entry;
    for counterpart_first in [true, false] {
        let (mut chat, mut rx, mut ops) = make_chatwidget_manual(None).await;
        let thread = ThreadId::new();
        chat.thread_id = Some(thread);
        let _ = drain_insert_history(&mut rx);
        let mut record = notice("grouped");
        record.origin_thread_id = thread.to_string();
        record.presentation.references =
            vec![codex_protocol::presentation::PresentationReference {
                kind: codex_protocol::presentation::PresentationReferenceKind::ExternalInput,
                id: "input-1".into(),
            }];
        record.semantic_sha256 = record.semantic_digest();
        record.receipt_id = record.receipt_digest();
        let input = Entry::Item { position:1, turn_id:"turn".into(), item:Box::new(AppServerThreadItem::FunctionCallOutput { external_input_view: None, id:"input-1".into(), name:"external_event".into(), namespace:Some("external".into()), output:codex_protocol::models::FunctionCallOutputBody::Text(json!({"source":{"kind":"service","id":"build"},"type":"message","text":"Original input body"}).to_string()) }) };
        let display = Entry::Presentation {
            position: 2,
            item: record.clone(),
        };
        let timeline = if counterpart_first {
            vec![input, display]
        } else {
            vec![display, input]
        };
        chat.replay_presentation_timeline(vec![], timeline.clone(), ReplayKind::ThreadSnapshot);
        let mut cells = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let AppEvent::InsertHistoryCell(cell) = event {
                cells.push(cell);
            }
        }
        assert_eq!(cells.len(), 1);
        let normal = lines_to_single_string(&cells[0].display_lines(40));
        let raw = lines_to_single_string(&cells[0].raw_lines());
        assert_eq!(normal.matches('•').count(), 1);
        assert!(raw.contains("Original input body"));
        assert!(raw.contains(&record.receipt_id));
        assert!(raw.contains("input-1"));
        insta::assert_snapshot!(format!("adjacent_group_{counterpart_first}"), normal);
        chat.replay_presentation_timeline(vec![], timeline, ReplayKind::ThreadSnapshot);
        assert!(drain_insert_history(&mut rx).is_empty());
        assert!(ops.try_recv().is_err());
    }
}

#[tokio::test]
async fn durable_notice_missing_nonadjacent_and_partial_refs() {
    use codex_app_server_protocol::ThreadTimelineEntry as Entry;
    for scenario in ["missing", "nonadjacent", "partial", "foreign-origin"] {
        let (mut chat, mut rx, _ops) = make_chatwidget_manual(None).await;
        let thread = ThreadId::new();
        chat.thread_id = Some(thread);
        let _ = drain_insert_history(&mut rx);
        let mut record = notice("linked");
        record.origin_thread_id = thread.to_string();
        record.presentation.references =
            vec![codex_protocol::presentation::PresentationReference {
                kind: codex_protocol::presentation::PresentationReferenceKind::ExternalInput,
                id: if scenario == "missing" {
                    "missing"
                } else {
                    "input"
                }
                .into(),
            }];
        if scenario == "partial" {
            record.presentation.references.push(
                codex_protocol::presentation::PresentationReference {
                    kind: codex_protocol::presentation::PresentationReferenceKind::McpInvocation,
                    id: "unloaded-call".into(),
                },
            );
        }
        if scenario == "foreign-origin" {
            record.origin_thread_id = "different-origin".into();
        }
        record.semantic_sha256 = record.semantic_digest();
        record.receipt_id = record.receipt_digest();
        let mut timeline=vec![Entry::Item {position:1,turn_id:"turn".into(),item:Box::new(AppServerThreadItem::FunctionCallOutput { external_input_view: None,id:"input".into(),name:"external_event".into(),namespace:Some("external".into()),output:codex_protocol::models::FunctionCallOutputBody::Text(json!({"source":{"kind":"service","id":"build"},"type":"message","text":"Independent body"}).to_string())})}];
        if scenario == "nonadjacent" {
            timeline.push(Entry::Presentation {
                position: 2,
                item: notice("unrelated"),
            });
        }
        timeline.push(Entry::Presentation {
            position: 3,
            item: record,
        });
        chat.replay_presentation_timeline(vec![], timeline, ReplayKind::ThreadSnapshot);
        let cells = drain_insert_history(&mut rx);
        assert_eq!(
            cells.len(),
            match scenario {
                "partial" => 1,
                "nonadjacent" => 3,
                _ => 2,
            }
        );
        let text = cells
            .into_iter()
            .map(|lines| lines_to_single_string(&lines))
            .collect::<String>();
        assert!(text.contains("Independent body"));
        if scenario == "partial" {
            assert!(text.contains("unloaded-call"));
        } else {
            assert!(text.contains("linked supplement"));
        }
    }
}

#[tokio::test]
async fn durable_notice_active_mcp_group_keeps_both_facts() {
    for early_flush in [false, true] {
        let (mut chat, mut rx, _ops) = make_chatwidget_manual(None).await;
        let thread = ThreadId::new();
        chat.thread_id = Some(thread);
        let _ = drain_insert_history(&mut rx);
        let mut item = AppServerThreadItem::McpToolCall {
            mcp_app_ui: None,
            id: "call".into(),
            server: "cutex_job".into(),
            tool: "query".into(),
            arguments: json!({"jobId":"job-1"}),
            status: codex_app_server_protocol::McpToolCallStatus::InProgress,
            app_context: None,
            mcp_app_resource_uri: None,
            plugin_id: None,
            read_only_hint: None,
            result: None,
            error: None,
            duration_ms: None,
        };
        chat.handle_mcp_tool_call_started_now(item.clone());
        let mut record = notice("live-linked");
        record.origin_thread_id = thread.to_string();
        record.presentation.references =
            vec![codex_protocol::presentation::PresentationReference {
                kind: codex_protocol::presentation::PresentationReferenceKind::McpInvocation,
                id: "call".into(),
            }];
        record.semantic_sha256 = record.semantic_digest();
        record.receipt_id = record.receipt_digest();
        chat.on_presentation(record.clone());
        assert!(drain_insert_history(&mut rx).is_empty());
        if early_flush {
            chat.add_info_message("Unrelated fact".into(), None);
            let prior = drain_insert_history(&mut rx)
                .into_iter()
                .map(|lines| lines_to_single_string(&lines))
                .collect::<String>();
            assert!(prior.contains("Report"));
        }
        if let AppServerThreadItem::McpToolCall { status, result, .. } = &mut item {
            *status = codex_app_server_protocol::McpToolCallStatus::Completed;
            *result = Some(Box::new(codex_app_server_protocol::McpToolCallResult {
                content: vec![
                    json!({"type":"text","text":json!({"schema":"cutex/job-service-core/v1","jobId":"job-1","revision":1,"state":"running"}).to_string()}),
                ],
                structured_content: None,
                meta: None,
            }));
        }
        chat.handle_mcp_tool_call_completed_now(item.clone());
        let mut cells = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let AppEvent::InsertHistoryCell(cell) = event {
                cells.push(cell);
            }
        }
        assert_eq!(cells.len(), 1);
        let normal = lines_to_single_string(&cells[0].display_lines(60));
        assert_eq!(normal.matches('•').count(), 1);
        assert!(normal.contains("Queried job"));
        let raw = lines_to_single_string(&cells[0].raw_lines());
        assert!(raw.contains("cutex_job.query"));
        if !early_flush {
            assert!(raw.contains(&record.receipt_id));
        }
        chat.handle_mcp_tool_call_completed_now(item);
        chat.on_presentation(record);
        assert!(drain_insert_history(&mut rx).is_empty());
    }
}
