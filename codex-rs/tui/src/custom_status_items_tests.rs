use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

fn document(text: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"version":1,"items":[
        {"id":"custom:profile","text":text,"style":{"fg":"#FFFFFF","bold":true}},
        {"id":"custom:bon-voyage","text":"Bon voyage !","style":{"fg":"#F6A3C8","bold":true}}
    ]}))
    .unwrap()
}

#[test]
#[allow(clippy::disallowed_methods)]
fn exact_styles_and_inherited_label_are_inert() {
    let items = StatusItems::parse(&document("  inherited\nlabel  ")).unwrap();
    assert_eq!(
        items.value(StatusLineItem::CustomProfile),
        "inherited label"
    );
    assert_eq!(items.value(StatusLineItem::CustomBonVoyage), "Bon voyage !");
    assert_eq!(
        items.style(StatusLineItem::CustomBonVoyage),
        Some(
            Style::default()
                .fg(Color::Rgb(246, 163, 200))
                .add_modifier(Modifier::BOLD)
        )
    );
    assert_eq!(
        items.style(StatusLineItem::CustomProfile),
        Some(
            Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .add_modifier(Modifier::BOLD)
        )
    );
    let inert = StatusItems::parse(&document("$(command) ${TOKEN} {account} /other/file")).unwrap();
    assert_eq!(
        inert.value(StatusLineItem::CustomProfile),
        "$(command) ${TOKEN} {account} /other/file"
    );
}

#[test]
fn bounds_controls_and_unsupported_sources() {
    assert!(StatusItems::parse(&document(&"é".repeat(128))).is_ok());
    assert!(StatusItems::parse(&document(&format!("{}x", "é".repeat(128)))).is_err());
    assert!(StatusItems::parse(&document("")).is_err());
    assert!(StatusItems::parse(&document("\n\u{1b}\u{202e}")).is_err());
    let items = StatusItems::parse(&document("abc\u{1b}[31m\u{202e}中文\r\nnext")).unwrap();
    assert_eq!(
        items.value(StatusLineItem::CustomProfile),
        "abc [31m 中文 next"
    );
    for bad in [
        json!({"version":2,"items":[]}),
        json!({"version":1.0,"items":[]}),
        json!({"version":1,"items":[],"source":"env"}),
        json!({"version":1,"items":[{"id":"custom:other","text":"x"}]}),
        json!({"version":1,"items":[{"id":"custom:profile","text":"x","source":{"kind":"launch_profile"}}]}),
        json!({"version":1,"items":[{"id":"custom:profile","text":"x","style":{"fg":"red"}}]}),
        json!({"version":1,"items":[{"id":"custom:profile","text":"x","style":{"bold":null}}]}),
        json!({"version":1,"items":[{"id":"custom:profile","text":"x"},{"id":"custom:profile","text":"y"}]}),
    ] {
        assert!(StatusItems::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
    }
    let mut exact = br#"{"version":1,"items":[]}"#.to_vec();
    exact.resize(MAX_FILE_BYTES as usize, b' ');
    assert!(StatusItems::parse(&exact).is_ok());
    exact.push(b' ');
    assert!(StatusItems::parse(&exact).is_err());
}

#[test]
fn selected_file_isolation_missing_and_malformed() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.json");
    let second = dir.path().join("second.json");
    std::fs::write(&first, document("first")).unwrap();
    std::fs::write(&second, document("second")).unwrap();
    let loaded = StatusItems::load(&first).unwrap();
    std::fs::write(&first, document("replacement")).unwrap();
    assert_eq!(loaded.value(StatusLineItem::CustomProfile), "first");
    assert_eq!(
        StatusItems::load(&second)
            .unwrap()
            .value(StatusLineItem::CustomProfile),
        "second"
    );
    assert!(StatusItems::load(&dir.path().join("missing")).is_err());
    assert!(StatusItems::load(Path::new("relative.json")).is_err());
    std::fs::write(&first, "malformed").unwrap();
    assert!(StatusItems::load(&first).is_err());
    assert!(StatusItems::load(dir.path()).is_err());
    #[cfg(unix)]
    {
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&second, &link).unwrap();
        assert!(StatusItems::load(&link).is_err());
    }
    assert_eq!(
        StatusItems::default().value(StatusLineItem::CustomProfile),
        "[cutex_profile unavailable]"
    );
    assert!("custom:other".parse::<StatusLineItem>().is_err());
}

#[test]
fn status_items_process_renderer() {
    // The real loaded-once store runs in a separate child so parallel/default
    // statusline snapshots cannot inherit a selected profile from this test.
    if let Some(path) = std::env::var_os("CODEX_TEST_STATUS_FIXTURE") {
        initialize(Some(Path::new(&path))).unwrap();
        std::fs::write(&path, document("changed-after-load")).unwrap();
        initialize(Some(Path::new(&path))).unwrap();
        assert_eq!(value(StatusLineItem::CustomProfile), "继承 profile");
        assert!(initialize(None).is_err());
        let line = crate::bottom_pane::status_line_from_segments(
            [
                (
                    StatusLineItem::CustomBonVoyage,
                    value(StatusLineItem::CustomBonVoyage),
                ),
                (
                    StatusLineItem::CustomProfile,
                    value(StatusLineItem::CustomProfile),
                ),
                (StatusLineItem::ModelName, "model low".to_owned()),
            ],
            /*use_theme_colors*/ false,
            /*thread_id*/ None,
        )
        .unwrap();
        assert_eq!(
            line.spans[0].style,
            style(StatusLineItem::CustomBonVoyage).unwrap()
        );
        assert_eq!(
            line.spans[2].style,
            style(StatusLineItem::CustomProfile).unwrap()
        );
        assert_eq!(line.spans[4].style, crate::style::secondary_text_style());
        let preview = crate::bottom_pane::StatusSurfacePreviewData::default()
            .status_line_for_items(
                [
                    StatusLineItem::CustomBonVoyage,
                    StatusLineItem::CustomProfile,
                ],
                /*use_theme_colors*/ true,
            )
            .unwrap();
        assert_eq!(preview.spans[..3], line.spans[..3]);
        // Render through the same native ratatui width handling as the footer.
        let draw = |width| {
            let mut buffer =
                ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, width, 1));
            ratatui::widgets::Widget::render(line.clone(), buffer.area, &mut buffer);
            let mut text = String::new();
            let mut x = 0;
            while x < width {
                let symbol = buffer[(x, 0)].symbol();
                text.push_str(symbol);
                x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
            }
            text.trim_end().to_owned()
        };
        insta::assert_snapshot!(
            "reviewed_status_items",
            format!("{}\n{}", draw(70), draw(24))
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("display.json");
    std::fs::write(&path, document("继承 profile")).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "custom_status_items::tests::status_items_process_renderer",
            "--nocapture",
        ])
        .env("CODEX_TEST_STATUS_FIXTURE", path)
        .env("CODEX_LAUNCH_PROFILE", "wrong-default-profile")
        .env(
            "CODEX_CUSTOM_STATUS_ITEMS_FILE",
            "/not-selected/catalog.json",
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn default_status_items_reentry_does_not_read_a_catalog() {
    initialize(None).unwrap();
    initialize(None).unwrap();
    assert_eq!(
        value(StatusLineItem::CustomProfile),
        "[cutex_profile unavailable]"
    );
    assert!(initialize(Some(Path::new("/not-selected.json"))).is_err());
}

#[test]
fn canonical_and_legacy_item_ids_share_values_and_duplicate_detection() {
    let old = document("configured profile");
    let new = String::from_utf8(old.clone())
        .unwrap()
        .replace("custom:profile", "cutex_profile")
        .replace("custom:bon-voyage", "cutex_welcome");
    let old = StatusItems::parse(&old).unwrap();
    let new = StatusItems::parse(new.as_bytes()).unwrap();
    for item in [
        StatusLineItem::CustomProfile,
        StatusLineItem::CustomBonVoyage,
    ] {
        assert_eq!(old.value(item), new.value(item));
        assert_eq!(old.style(item), new.style(item));
    }
    assert!(StatusItems::parse(br#"{"version":1,"items":[{"id":"custom:profile","text":"old"},{"id":"cutex_profile","text":"new"}]}"#).is_err());
}

#[test]
fn malformed_animation_keeps_the_static_fallback() {
    let bytes = serde_json::to_vec(&json!({"version":1,"items":[{"id":"cutex_welcome","text":"旅途愉快","animation":{"frames":[]}}]})).unwrap();
    let items = StatusItems::parse(&bytes).unwrap();
    assert_eq!(items.value(StatusLineItem::CustomBonVoyage), "旅途愉快");
}

#[test]
fn display_aliases_preserve_exact_model_ids() {
    let bytes = serde_json::to_vec(&json!({"version":1,"items":[],"model_display_names":{"gpt-6-astra":"astra","sonnet":"Sonnet"}})).unwrap();
    let items = StatusItems::parse(&bytes).unwrap();
    assert_eq!(
        items.1.get("gpt-6-astra").map(String::as_str),
        Some("astra")
    );
    assert_eq!(items.1.get("astra"), None);
    insta::assert_snapshot!(
        "model_display_aliases",
        items
            .1
            .iter()
            .map(|(id, display)| format!("{id} → {display}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let bad = serde_json::to_vec(
        &json!({"version":1,"items":[],"model_display_names":{"gpt-6-astra":"bad\u{001b}"}}),
    )
    .unwrap();
    assert!(StatusItems::parse(&bad).is_err());
}

#[test]
fn welcome_indentation_matches_animated_columns_without_visible_text() {
    use ratatui::text::Line;
    use ratatui::text::Span;
    use std::time::Duration;
    let bytes = serde_json::to_vec(&json!({"version":1,"items":[{
        "id":"cutex_welcome","text":"旅途愉快","animation":{
            "width":12,"repeat":true,"frames":[
                {"duration_ms":200,"spans":[{"text":"lv"}]},
                {"duration_ms":200,"spans":[{"text":"旅途愉快"}]}
            ]}
    }]}))
    .unwrap();
    let items = StatusItems::parse(&bytes).unwrap();
    for elapsed in [Duration::ZERO, Duration::from_millis(250)] {
        let mut greeting = Line::from("旅途愉快");
        animate_line_with_items(
            &mut greeting,
            &[StatusLineItem::CustomBonVoyage],
            &items,
            elapsed,
        );
        let mut indent = Line::from(vec![Span::raw("        "), Span::raw(""), Span::raw("sol")]);
        animate_line_with_items(
            &mut indent,
            &[
                StatusLineItem::CutexWelcomeIndentation,
                StatusLineItem::ModelName,
            ],
            &items,
            elapsed,
        );
        assert_eq!(greeting.width(), 12);
        assert_eq!(indent.to_string(), "            sol");
    }
}

#[test]
fn aligned_profile_centers_and_truncates_by_terminal_columns() {
    assert_eq!(centered_profile("abc", 8), "  abc   ");
    assert_eq!(centered_profile("中文", 8), "  中文  ");
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(centered_profile("long profile", 4).as_str()),
        4
    );
    assert_eq!(centered_profile("profile", 0), "");
}

#[test]
fn animated_welcome_and_aligned_profile_keep_matching_widths_and_profile_style() {
    use ratatui::text::Line;
    use ratatui::text::Span;
    let items=StatusItems::parse(&serde_json::to_vec(&json!({"version":1,"items":[
        {"id":"cutex_profile","text":"中文","style":{"fg":"#E08EB2"}},
        {"id":"cutex_welcome","text":"hello","animation":{"width":12,"repeat":true,"frames":[{"duration_ms":200,"spans":[{"text":"lv"}]},{"duration_ms":200,"spans":[{"text":"旅途愉快"}]}]}}
    ]})).unwrap()).unwrap();
    let style = items.style(StatusLineItem::CustomProfile).unwrap();
    let mut line = Line::from(vec![
        Span::styled("中文", style),
        Span::raw(" · "),
        Span::raw("W 1d03h 16%"),
    ]);
    animate_line_with_items(
        &mut line,
        &[
            StatusLineItem::CutexAlignedProfile,
            StatusLineItem::CutexSimpleWeeklyLimit,
        ],
        &items,
        std::time::Duration::ZERO,
    );
    assert_eq!(line.spans[0].content, "    中文    ");
    assert_eq!(line.spans[0].style, style);
    assert!(
        line.spans[2]
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
    assert_eq!(line.to_string(), "    中文     · W 1d03h 16%");
}
