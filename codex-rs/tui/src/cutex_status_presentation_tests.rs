use super::*;
use pretty_assertions::assert_eq;
#[test]
fn quarter_cells_and_compact_labels_preserve_remaining_semantics() {
    assert_eq!(
        format_quota(35.0, Some(15780), 0, 10),
        "W[██████▌   ]65% 4h23m"
    );
    assert_eq!(format_quota(100.0, None, 0, 10), "W[          ]0% —");
    assert_eq!(format_quota(0.0, Some(-1), 0, 10), "W[██████████]100% 0m");
    assert_eq!(
        format_quota(67.0, Some(529200), 0, 10),
        "W[███▎      ]33% 6d03h"
    );
    for n in 0..=100 {
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(
                format_quota(n as f64, None, 0, 10)
                    .split_once('[')
                    .unwrap()
                    .1
                    .split_once(']')
                    .unwrap()
                    .0
            ),
            10
        );
    }
}
#[test]
fn aliases_are_component_bounded_longest_and_non_recursive() {
    let p = vec![
        Prefix {
            from: "~/Projects".into(),
            to: "~~".into(),
        },
        Prefix {
            from: "~/Projects/IFM".into(),
            to: "IFM".into(),
        },
    ];
    assert_eq!(alias("~/Projects2/a", &p), "~/Projects2/a");
    assert_eq!(alias("~/Projects/IFM/a", &p), "IFM/a");
    assert_eq!(alias("~\\Projects\\a", &p), "~~\\a");
}

#[test]
fn unsaved_profile_inherits_shared_presentation_until_first_save() {
    let root = tempfile::tempdir().unwrap();
    let shared = root.path().join("status-line.toml");
    let profiles = root.path().join("status-lines");
    std::fs::create_dir(&profiles).unwrap();
    let profile = profiles.join("example.toml");
    std::fs::write(&shared, "shared-style").unwrap();
    assert_eq!(read_preferences(&profile).unwrap(), "shared-style");
    std::fs::write(&profile, "profile-style").unwrap();
    assert_eq!(read_preferences(&profile).unwrap(), "profile-style");
    assert!(read_preferences(&root.path().join("missing.toml")).is_err());
}

#[test]
fn simple_weekly_countdown_uses_compact_single_spaces() {
    let values = [
        format_quota(50.0, Some(529200), 0, 0),
        format_quota(50.0, Some(15780), 0, 0),
        format_quota(50.0, Some(-1), 0, 0),
        format_quota(50.0, None, 0, 0),
    ];
    insta::assert_snapshot!(values.join("\n"), @"
    W 6d03h 50%
    W 4h23m 50%
    W 0m 50%
    W — 50%
    ");
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "verify that styling retains the existing RGB color"
)]
fn identifier_only_is_underlined_without_changing_width_or_body_style() {
    use ratatui::text::Span;
    let style = Style::default().fg(Color::Rgb(224, 142, 178));
    for (item, text) in [
        (StatusLineItem::CutexSimpleFiveHourLimit, "5 3h20m 98%"),
        (StatusLineItem::CutexSimpleWeeklyLimit, "W 1d03h 16%"),
        (StatusLineItem::CutexFableLimit, "F 1d03h 16%"),
        (StatusLineItem::CutexContextRemaining, "C 72%"),
    ] {
        let spans = decorate_span(item, Span::styled(text.to_owned(), style));
        assert_eq!(
            spans.iter().map(|s| s.content.as_ref()).collect::<String>(),
            text
        );
        assert!(spans[0].style.add_modifier.contains(Modifier::UNDERLINED));
        assert_eq!(spans[1].style, style);
    }
}

#[test]
fn presentation_parses_a_toml_document_with_sections_and_aliases() {
    let parsed = parse_items(
        r##"
[tui]
status_line = ["cutex_workdir"]
[tui.presentation.cutex_workdir]
aliases = [{ from = "~/Developments", to = "$DEV", bold = true }]
[tui.presentation.cutex_workdir.style]
fg = "#E08EB2"
[tui.presentation.cutex_weekly_limit]
bar_width = 8
"##,
    );
    let workdir = &parsed["cutex_workdir"];
    assert_eq!(
        crate::cutex_workdir::replace("~/Developments/demo", &workdir.aliases),
        "$DEV/demo"
    );
    assert_eq!(workdir.style.fg.as_deref(), Some("#E08EB2"));
    assert_eq!(parsed["cutex_weekly_limit"].bar_width, Some(8));
}
