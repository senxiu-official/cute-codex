use super::*;
use pretty_assertions::assert_eq;

#[test]
fn helper_response_renders_remaining_countdown_and_stale_data() {
    let mut d = parse_response(
        br#"{"custom_limit":{"label":"Q","remaining_percent":75,"reset_at":97200,"stale":false}}"#,
    )
    .unwrap();
    let mut rows = vec![
        d.render(StatusLineItem::CutexCustomizedLimit, 0),
        d.render(StatusLineItem::CutexCustomizedLimitBar, 0),
        d.render(StatusLineItem::CutexCustomizedLimit, 3600),
    ];
    d.stale = true;
    rows.push(d.render(StatusLineItem::CutexCustomizedLimit, 3600));
    d.remaining_percent = None;
    rows.push(d.render(StatusLineItem::CutexCustomizedLimit, 3600));
    insta::assert_snapshot!(rows.join("\n"), @"
    Q 1d03h 75%
    Q[███████▌  ]75% 1d03h
    Q 1d02h 75%
    Q~ 1d02h 75%
    Q~ —
    ");
}

#[test]
fn missing_and_invalid_quota_do_not_affect_notification_response() {
    assert!(parse_response(br#"{"label":"OFF"}"#).is_none());
    assert!(parse_response(br#"{"custom_limit":null}"#).is_none());
    assert!(
        parse_response(
            br#"{"custom_limit":{"label":"\u001b","remaining_percent":50,"stale":false}}"#
        )
        .is_none()
    );
    let d =
        parse_response(br#"{"custom_limit":{"label":"Q","remaining_percent":150,"stale":false}}"#)
            .unwrap();
    assert_eq!(
        d.render(StatusLineItem::CutexCustomizedLimit, 0),
        "Q — 100%"
    );
}
