//! Local, loaded-once display policy. Never changes quota data or runtime configuration.
use crate::bottom_pane::StatusLineItem;
use crate::status::RateLimitWindowDisplay;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Presentation {
    bar_width: Option<usize>,
    style: ItemStyle,
    prefix: Vec<Prefix>,
    aliases: Vec<crate::cutex_workdir::Alias>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ItemStyle {
    fg: Option<String>,
    bg: Option<String>,
    bold: Option<bool>,
    dim: Option<bool>,
}
#[derive(Deserialize)]
struct Prefix {
    from: String,
    to: String,
}
static ITEMS: OnceLock<BTreeMap<String, Presentation>> = OnceLock::new();
fn items() -> &'static BTreeMap<String, Presentation> {
    ITEMS.get_or_init(|| {
        if cfg!(test) {
            return BTreeMap::new();
        }
        let path = std::env::var_os("CUTEX_STATUS_LINE_CONFIG")
            .map(std::path::PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".cutex/status-line.toml")));
        let Some(path) = path else {
            return BTreeMap::new();
        };
        let Ok(text) = read_preferences(&path) else {
            return BTreeMap::new();
        };
        parse_items(&text)
    })
}

fn parse_items(text: &str) -> BTreeMap<String, Presentation> {
    let Ok(value) = toml::from_str::<toml::Value>(text) else {
        return BTreeMap::new();
    };
    let Some(table) = value
        .get("tui")
        .and_then(|v| v.get("presentation"))
        .and_then(toml::Value::as_table)
    else {
        return BTreeMap::new();
    };
    table
        .iter()
        .filter_map(|(id, v)| match v.clone().try_into() {
            Ok(p) => Some((id.clone(), p)),
            Err(error) => {
                tracing::warn!(%id, %error, "Ignoring invalid statusline presentation item");
                None
            }
        })
        .collect()
}

// Match Cutex's initial profile inheritance until that profile first saves its own file.
fn read_preferences(path: &std::path::Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && path
                    .parent()
                    .and_then(std::path::Path::file_name)
                    .is_some_and(|n| n == "status-lines") =>
        {
            let shared = path
                .parent()
                .and_then(std::path::Path::parent)
                .map(|root| root.join("status-line.toml"));
            match shared {
                Some(shared) => std::fs::read_to_string(shared),
                None => Err(error),
            }
        }
        result => result,
    }
}

pub(crate) fn quota(
    window: &RateLimitWindowDisplay,
    item: StatusLineItem,
    now: i64,
) -> Option<String> {
    if !window.used_percent.is_finite() {
        return None;
    }
    let prefix = match item {
        StatusLineItem::CutexFiveHourLimit | StatusLineItem::CutexSimpleFiveHourLimit => "5",
        StatusLineItem::CutexFableLimit | StatusLineItem::CutexFableLimitBar => "F",
        _ => "W",
    };
    let width = if matches!(
        item,
        StatusLineItem::CutexSimpleWeeklyLimit
            | StatusLineItem::CutexSimpleFiveHourLimit
            | StatusLineItem::CutexFableLimit
    ) {
        0
    } else {
        bar_width(item)
    };
    Some(
        format_quota(window.used_percent, window.resets_at_unix, now, width)
            .replacen("W", prefix, 1),
    )
}
pub(crate) fn bar_width(item: StatusLineItem) -> usize {
    items()
        .get(&item.to_string())
        .and_then(|p| p.bar_width)
        .unwrap_or(10)
        .clamp(1, 40)
}
pub(crate) fn format_quota(used: f64, reset: Option<i64>, now: i64, width: usize) -> String {
    let remaining = (100.0 - used).clamp(0.0, 100.0);
    let quarters = (remaining * (width * 4) as f64 / 100.0).round() as usize;
    let full = quarters / 4;
    let partial = ["", "▎", "▌", "▊"][quarters % 4];
    let empty = width.saturating_sub(full + usize::from(!partial.is_empty()));
    let clock = reset
        .map(|t| {
            let minutes = t.saturating_sub(now).max(0).saturating_add(59) / 60;
            if minutes >= 1440 {
                format!("{}d{:02}h", minutes / 1440, minutes / 60 % 24)
            } else if minutes >= 60 {
                format!("{}h{:02}m", minutes / 60, minutes % 60)
            } else {
                format!("{minutes}m")
            }
        })
        .unwrap_or_else(|| "—".into());
    let clock = if clock.chars().count() > 6 {
        ">999d".into()
    } else {
        clock
    };
    if width == 0 {
        return format!("W {clock} {remaining:.0}%");
    }
    format!(
        "W[{}{partial}{}]{remaining:.0}% {clock}",
        "█".repeat(full),
        " ".repeat(empty)
    )
}
pub(crate) fn text(item: StatusLineItem, value: String) -> String {
    if item == StatusLineItem::CutexWorkdir {
        return items().get("cutex_workdir").map_or(value.clone(), |p| {
            crate::cutex_workdir::replace(&value, &p.aliases)
        });
    }
    if item != StatusLineItem::CurrentDir {
        return value;
    }
    let Some(p) = items().get(&item.to_string()) else {
        return value;
    };
    alias(&value, &p.prefix)
}
fn alias(value: &str, prefixes: &[Prefix]) -> String {
    let normalized = value.replace('\\', "/");
    prefixes
        .iter()
        .filter_map(|p| {
            let from = p.from.replace('\\', "/").trim_end_matches('/').to_string();
            (!from.is_empty()
                && normalized.starts_with(&from)
                && (normalized.len() == from.len()
                    || normalized.as_bytes().get(from.len()) == Some(&b'/')))
            .then_some((p, from.len()))
        })
        .max_by_key(|(_, len)| *len)
        .map_or_else(
            || value.into(),
            |(p, len)| format!("{}{}", p.to, &value[len..]),
        )
}
pub(crate) fn style(item: StatusLineItem, base: Style) -> Style {
    let Some(p) = items().get(&item.to_string()) else {
        return base;
    };
    let mut result = base;
    for (value, foreground) in [(&p.style.fg, true), (&p.style.bg, false)] {
        if let Some(value) = value
            && let Some(color) = color(value)
        {
            result = if foreground {
                result.fg(color)
            } else {
                result.bg(color)
            };
        }
    }
    for (enabled, modifier) in [(p.style.bold, Modifier::BOLD), (p.style.dim, Modifier::DIM)] {
        if let Some(enabled) = enabled {
            result = if enabled {
                result.add_modifier(modifier)
            } else {
                result.remove_modifier(modifier)
            };
        }
    }
    result
}
#[expect(
    clippy::disallowed_methods,
    reason = "preserve explicit user-provided status RGB colors"
)]
fn color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

#[cfg(test)]
#[path = "cutex_status_presentation_tests.rs"]
mod tests;

/// Decorate a segment after item-wide styles, preserving other segments' identities.
pub(crate) fn decorate_span(
    item: StatusLineItem,
    span: ratatui::text::Span<'static>,
) -> Vec<ratatui::text::Span<'static>> {
    if item == StatusLineItem::CutexAgentName {
        return crate::cutex_agent_status::spans(span);
    }
    if item == StatusLineItem::CutexWorkdir {
        return items().get("cutex_workdir").map_or_else(
            || vec![span.clone()],
            |p| crate::cutex_workdir::styled(&span.content, span.style, &p.aliases),
        );
    }
    if matches!(
        item,
        StatusLineItem::CutexSimpleWeeklyLimit
            | StatusLineItem::CutexSimpleFiveHourLimit
            | StatusLineItem::CutexWeeklyLimit
            | StatusLineItem::CutexFiveHourLimit
            | StatusLineItem::CutexFableLimit
            | StatusLineItem::CutexFableLimitBar
            | StatusLineItem::CutexCustomizedLimit
            | StatusLineItem::CutexCustomizedLimitBar
            | StatusLineItem::CutexContextRemaining
    ) && span.content.len() >= 2
    {
        return vec![
            ratatui::text::Span::styled(
                span.content[..1].to_owned(),
                span.style.add_modifier(Modifier::UNDERLINED),
            ),
            ratatui::text::Span::styled(span.content[1..].to_owned(), span.style),
        ];
    }
    vec![span]
}
pub(crate) fn decorate_line(line: &mut ratatui::text::Line<'static>, ids: &[StatusLineItem]) {
    line.spans = std::mem::take(&mut line.spans)
        .into_iter()
        .enumerate()
        .flat_map(|(index, span)| {
            if index % 2 == 0
                && let Some(id) = ids.get(index / 2)
            {
                decorate_span(*id, span)
            } else {
                vec![span]
            }
        })
        .collect();
}
