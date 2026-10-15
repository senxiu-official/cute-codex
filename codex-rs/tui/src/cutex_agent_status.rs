//! Read the launcher's display-only Agent metadata once, without querying a service.
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Span;
use serde::Deserialize;
use std::sync::OnceLock;
#[derive(Deserialize)]
struct Agent {
    name: String,
    badge: Option<Badge>,
}
#[derive(Deserialize)]
struct Badge {
    label: String,
    color: String,
}
static AGENT: OnceLock<Option<Agent>> = OnceLock::new();
fn parse(text: &str) -> Option<Agent> {
    if text.len() > 4096 {
        return None;
    }
    let a: Agent = serde_json::from_str(text).ok()?;
    if a.name.is_empty() || a.name.len() > 256 || a.name.chars().any(char::is_control) {
        return None;
    }
    if let Some(b) = &a.badge
        && (b.label.len() > 32 || b.label.chars().any(char::is_control))
    {
        return None;
    }
    Some(a)
}
fn agent() -> Option<&'static Agent> {
    AGENT
        .get_or_init(|| {
            std::env::var("CUTEX_AGENT_STATUS")
                .ok()
                .and_then(|s| parse(&s))
        })
        .as_ref()
}
pub(crate) fn value() -> Option<String> {
    agent().map(|a| match &a.badge {
        Some(b) => format!(" {}  {}", b.label, a.name),
        None => a.name.clone(),
    })
}
pub(crate) fn spans(span: Span<'static>) -> Vec<Span<'static>> {
    let Some(a) = agent() else {
        return vec![span];
    };
    let Some(b) = &a.badge else {
        return vec![span];
    };
    let Some(style) = badge_style(&b.color) else {
        return vec![span];
    };
    vec![
        Span::styled(format!(" {} ", b.label), style),
        Span::styled(format!(" {}", a.name), span.style),
    ]
}
#[expect(
    clippy::disallowed_methods,
    reason = "preserve explicit user-provided badge RGB colors"
)]
fn badge_style(token: &str) -> Option<Style> {
    let (bg, (r, g, b)) = match token {
        "cyan" => (Color::Cyan, (0, 128, 128)),
        "blue" => (Color::LightBlue, (0, 0, 255)),
        "green" => (Color::LightGreen, (0, 255, 0)),
        "magenta" => (Color::LightMagenta, (255, 0, 255)),
        "yellow" => (Color::Yellow, (128, 128, 0)),
        "red" => (Color::LightRed, (255, 0, 0)),
        other => {
            let Color::Rgb(r, g, b) = crate::custom_status_items::parse_color(other).ok()? else {
                return None;
            };
            (Color::Rgb(r, g, b), (r, g, b))
        }
    };
    let linear = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
    let fg = if (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) {
        Color::Black
    } else {
        Color::White
    };
    Some(Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "verify exact configured badge color"
    )]
    fn missing_identity_is_not_replaced_and_badge_preserves_exact_color() {
        assert!(parse("{}").is_none());
        let a = parse(r##"{"name":"director","badge":{"label":"IF","color":"#74BAC3"}}"##).unwrap();
        assert_eq!(a.name, "director");
        let style = badge_style(&a.badge.unwrap().color).unwrap();
        assert_eq!(style.bg, Some(Color::Rgb(116, 186, 195)));
        assert_eq!(style.fg, Some(Color::Black));
    }
}
