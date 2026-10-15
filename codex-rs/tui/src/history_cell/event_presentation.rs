//! Shared presentation only: callers own event identity, persistence, and status semantics.
use super::messages::sanitize_user_text;
use super::*;

/// Render an optional header/body with an optional native bullet. Text is always data.
pub(super) fn render_event(
    header: Option<&str>,
    body: &[String],
    bullet: Option<Span<'static>>,
    width: u16,
) -> Vec<Line<'static>> {
    let indent = if bullet.is_some() { "  " } else { "" };
    let mut lines = Vec::new();
    if let Some(header) = header {
        let clean = sanitize_user_text(header.into());
        let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut prefix = Vec::new();
        if let Some(bullet) = bullet {
            prefix.extend([bullet, " ".into()]);
        }
        let header = Line::from(clean.bold());
        lines.extend(
            adaptive_wrap_line(
                &header,
                RtOptions::new(usize::from(width).max(1))
                    .initial_indent(Line::from(prefix))
                    .subsequent_indent(indent.into()),
            )
            .into_iter()
            .map(|line| line_to_static(&line)),
        );
    }
    for text in body {
        let clean = sanitize_user_text(text.as_str().into());
        for part in clean.lines() {
            lines.extend(
                adaptive_wrap_line(
                    &Line::from(part.to_owned()),
                    RtOptions::new(usize::from(width).max(1))
                        .initial_indent(indent.into())
                        .subsequent_indent(indent.into()),
                )
                .into_iter()
                .map(|line| line_to_static(&line)),
            );
        }
    }
    lines
}

/// Bound the display preview while keeping the retained transcript intact.
pub(super) fn preview(text: &str, max_lines: usize, line_width: usize) -> String {
    let max_graphemes = max_lines
        .saturating_mul(line_width)
        .saturating_sub(max_lines);
    let formatted = crate::text_formatting::format_json_compact(text);
    crate::text_formatting::truncate_text(formatted.as_deref().unwrap_or(text), max_graphemes)
}
