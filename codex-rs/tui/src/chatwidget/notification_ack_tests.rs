use super::*;

#[test]
fn typing_coalesces_and_new_reminder_replaces_pending() {
    let mut ack = NotificationAck::default();
    ack.interact();
    assert_eq!(ack.pending, None);
    ack.observe(Some("first".into()));
    for _ in 0..100 {
        ack.interact();
    }
    assert_eq!(ack.pending.as_deref(), Some("first"));
    ack.submitted = ack.pending.take();
    for _ in 0..100 {
        ack.interact();
    }
    assert_eq!(ack.pending, None);
    ack.observe(Some("second".into()));
    ack.interact();
    assert_eq!(ack.pending.as_deref(), Some("second"));
    ack.observe(None);
    assert_eq!(ack.pending, None);
}

#[tokio::test]
async fn failure_retries_after_delay_but_stops_after_three_attempts() {
    let (draw, _) = tokio::sync::broadcast::channel(1);
    let frame = crate::tui::FrameRequester::new(draw);
    for attempts in [1, 3] {
        let (tx, rx) = mpsc::channel();
        tx.send(Err("unavailable".into())).unwrap();
        let mut ack = NotificationAck {
            current: Some("one".into()),
            submitted: Some("one".into()),
            response: Some(rx),
            attempts,
            ..Default::default()
        };
        ack.poll("session", &frame);
        assert_eq!(ack.pending.is_some(), attempts < 3);
        assert_eq!(ack.retry_at.is_some(), attempts < 3);
        for _ in 0..100 {
            ack.interact();
        }
        assert_eq!(ack.attempts, attempts);
    }
}

#[tokio::test]
async fn old_failure_does_not_acknowledge_a_replacement_without_input() {
    let (draw, _) = tokio::sync::broadcast::channel(1);
    let frame = crate::tui::FrameRequester::new(draw);
    let (tx, rx) = mpsc::channel();
    tx.send(Err("old failure".into())).unwrap();
    let mut ack = NotificationAck {
        current: Some("new".into()),
        submitted: Some("old".into()),
        response: Some(rx),
        ..Default::default()
    };
    ack.poll("session", &frame);
    assert!(ack.pending.is_none());
    assert!(ack.response.is_none());
}

#[test]
fn input_before_poll_only_acknowledges_reminders_created_before_input() {
    let at = chrono::DateTime::parse_from_rfc3339("2026-09-15T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let mut ack = NotificationAck {
        last_interaction: Some(at),
        ..Default::default()
    };
    ack.observe_at(
        Some("older".into()),
        Some(at - chrono::Duration::milliseconds(1)),
    );
    assert_eq!(ack.pending.as_deref(), Some("older"));
    ack.observe_at(
        Some("newer".into()),
        Some(at + chrono::Duration::milliseconds(1)),
    );
    assert!(ack.pending.is_none());
    ack.observe_at(Some("legacy".into()), None);
    assert!(ack.pending.is_none());
}
