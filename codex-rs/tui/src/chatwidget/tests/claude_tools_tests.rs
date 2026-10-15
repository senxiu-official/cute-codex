use super::*;
use serde_json::json;

#[tokio::test]
async fn claude_search_renders_live_and_replayed_without_executing_a_tool() {
    for replay in [false, true] {
        let (mut chat, mut rx, _ops) = make_chatwidget_manual(None).await;
        chat.note_rendered_width(80);
        let item = |status| {
            serde_json::from_value::<ThreadItem>(json!({
                "type":"dynamicToolCall","id":"native-search","namespace":"claude",
                "tool":"WebSearch","arguments":{"query":"Claude SDK documentation"},
                "status":status,"contentItems":[],"success":true,"durationMs":100
            }))
            .unwrap()
        };
        if replay {
            chat.replay_thread_item(item("completed"), "turn".into(), ReplayKind::ThreadSnapshot);
        } else {
            chat.handle_server_notification(
                ServerNotification::ItemStarted(ItemStartedNotification {
                    thread_id: "thread-1".into(),
                    turn_id: "turn".into(),
                    item: item("inProgress"),
                    started_at_ms: 0,
                }),
                None,
            );
            chat.handle_server_notification(
                ServerNotification::ItemCompleted(ItemCompletedNotification {
                    thread_id: "thread-1".into(),
                    turn_id: "turn".into(),
                    item: item("completed"),
                    completed_at_ms: 0,
                }),
                None,
            );
        }
        let rendered = drain_insert_history(&mut rx)
            .into_iter()
            .flatten()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("Claude SDK documentation"), "{rendered}");
        assert!(rendered.contains("Searched"), "{rendered}");
    }
}

#[tokio::test]
async fn fable_quota_is_independent_and_never_falls_back_to_weekly() {
    let (mut chat, _rx, _ops) = make_chatwidget_manual(None).await;
    chat.config.model_provider_id = "cute-claude".into();
    for (id, label, used) in [
        ("claude-seven_day", "Claude 7d", 20),
        ("claude-model-fable", "Claude Fable 7d", 70),
    ] {
        let raw=serde_json::from_value(json!({"limitId":id,"limitName":label,"primary":{"usedPercent":used,"windowDurationMins":10080,"resetsAt":null},"secondary":null,"credits":null,"planType":null})).unwrap();
        chat.rate_limit_snapshots_by_limit_id.insert(
            id.into(),
            crate::status::rate_limit_snapshot_display_for_limit(
                &raw,
                label.into(),
                chrono::Local::now(),
                crate::clock_format::ClockFormat::TwentyFourHour,
            ),
        );
    }
    assert_eq!(
        chat.status_line_value(StatusLineItem::CutexSimpleWeeklyLimit),
        Some("W — 80%".into())
    );
    assert_eq!(
        chat.status_line_value(StatusLineItem::CutexFableLimit),
        Some("F — 30%".into())
    );
    assert_eq!(
        chat.status_line_value(StatusLineItem::CutexFableLimitBar),
        Some("F[███       ]30% —".into())
    );
    chat.rate_limit_snapshots_by_limit_id
        .remove("claude-model-fable");
    assert_eq!(
        chat.status_line_value(StatusLineItem::CutexFableLimit),
        None
    );
}
