use super::*;

#[test]
fn notification_response_is_bound_to_requested_session() {
    assert_eq!(
        parse_response("a", br#"{"thread_id":"a","label":"CIAO!"}"#),
        Ok("CIAO!".into())
    );
    assert!(parse_response("b", br#"{"thread_id":"a","label":"ON"}"#).is_err());
    assert!(parse_response("a", br#"{"thread_id":"a","label":"\u001b[31m"}"#).is_err());
}

#[tokio::test]
async fn notification_item_uses_confirmed_labels_in_preview() {
    let (mut chat, _rx, _op_rx, _sender) =
        crate::chatwidget::tests::make_chatwidget_manual_with_sender().await;
    let mut lines = Vec::new();
    for label in ["CIAO!", "ON", "OFF", "关注", "NOTIFY?"] {
        chat.notification_control.label = Some(label.into());
        let line = chat
            .status_surface_preview_data()
            .status_line_for_items(
                [StatusLineItem::Notification],
                /*use_theme_colors*/ true,
            )
            .unwrap();
        lines.push(
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
        );
    }
    insta::assert_snapshot!(lines.join("\n"), @"
    CIAO!
    ON
    OFF
    关注
    NOTIFY?
    ");
}

#[tokio::test]
async fn notification_styles_are_exact_and_independent_of_label_text() {
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    let (mut chat, _sender, _rx, _op_rx) =
        crate::chatwidget::tests::make_chatwidget_manual_with_sender().await;
    let mut rendered = Vec::new();
    for (color, bold) in [
        ("#E08EB2", true),
        ("#74BAC3", false),
        ("#D9B45F", false),
        ("#123456", true),
    ] {
        let bytes =
            serde_json::to_vec(&serde_json::json!({"style":{"fg":color,"bold":bold}})).unwrap();
        chat.notification_control.label = Some("same label".into());
        chat.notification_control.style = Some(parse_style(&bytes).unwrap());
        let line = chat
            .status_surface_preview_data()
            .status_line_for_items(
                [StatusLineItem::Notification],
                /*use_theme_colors*/ true,
            )
            .unwrap();
        let span = &line.spans[0];
        assert_eq!(span.style.add_modifier.contains(Modifier::BOLD), bold);
        let Some(Color::Rgb(r, g, b)) = span.style.fg else {
            panic!("exact configured RGB required")
        };
        rendered.push(format!(
            "{} #{r:02X}{g:02X}{b:02X} bold={bold}",
            span.content
        ));
    }
    insta::assert_snapshot!(rendered.join("\n"), @"
    same label #E08EB2 bold=true
    same label #74BAC3 bold=false
    same label #D9B45F bold=false
    same label #123456 bold=true
    ");
    assert!(parse_style(br##"{"style":{"fg":"red"}}"##).is_err());
}
