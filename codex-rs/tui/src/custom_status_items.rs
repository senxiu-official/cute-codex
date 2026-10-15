//! Loaded-once inert status-line values and optional precompiled animations.
//!
//! These labels are presentation only. No environment, account, repository config,
//! model context, or runtime source resolution participates in their interpretation.
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use serde::Deserialize;

use crate::bottom_pane::StatusLineItem;

const MAX_FILE_BYTES: u64 = 1_048_576;
static ORIGIN: OnceLock<std::time::Instant> = OnceLock::new();
static ITEMS: OnceLock<(Option<PathBuf>, StatusItems)> = OnceLock::new();

#[derive(Default)]
pub(crate) struct StatusItems(
    BTreeMap<String, (String, Style, Option<crate::status_animation::Animation>)>,
    BTreeMap<String, String>,
);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    items: Vec<Item>,
    #[serde(default)]
    model_display_names: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Item {
    id: String,
    text: String,
    #[serde(default)]
    style: ItemStyle,
    #[serde(default)]
    animation: Option<serde_json::Value>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ItemStyle {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    dim: bool,
    italic: bool,
    underlined: bool,
}

pub(crate) fn initialize(path: Option<&Path>) -> io::Result<()> {
    // Upstream local-database recovery can re-enter the TUI. Reuse the frozen
    // selection, including the default, without reading changed display bytes.
    if let Some((selected, _)) = ITEMS.get() {
        return if selected.as_deref() == path {
            Ok(())
        } else {
            Err(invalid(
                "status items selection cannot change in this process",
            ))
        };
    }
    let items = path.map(StatusItems::load).transpose()?.unwrap_or_default();
    ORIGIN.get_or_init(std::time::Instant::now);
    ITEMS
        .set((path.map(Path::to_path_buf), items))
        .map_err(|_| io::Error::other("status items already initialized"))
}

/// Presentation aliases never replace the model ID used by the backend.
pub(crate) fn model_display_name(model: &str) -> Option<&'static str> {
    ITEMS
        .get()
        .and_then(|(_, items)| items.1.get(model).map(String::as_str))
}

pub(crate) fn value(item: StatusLineItem) -> String {
    ITEMS.get().map_or_else(
        || format!("[{item} unavailable]"),
        |(_, items)| items.value(item),
    )
}

pub(crate) fn style(item: StatusLineItem) -> Option<Style> {
    ITEMS.get().and_then(|(_, items)| items.style(item))
}

pub(crate) fn is_custom(item: StatusLineItem) -> bool {
    matches!(
        item,
        StatusLineItem::CustomProfile
            | StatusLineItem::CustomBonVoyage
            | StatusLineItem::CutexAlignedProfile
    )
}

impl StatusItems {
    fn load(path: &Path) -> io::Result<Self> {
        if !path.is_absolute() || !path.symlink_metadata()?.file_type().is_file() {
            return Err(invalid(
                "status items require an absolute regular nonsymlink file",
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(path)?;
        if !file.metadata()?.is_file() {
            return Err(invalid("status items require a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        Self::parse(&bytes)
    }

    fn parse(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > MAX_FILE_BYTES as usize {
            return Err(invalid("status items file exceeds 1048576 bytes"));
        }
        let document: Document = serde_json::from_slice(bytes)
            .map_err(|_| invalid("invalid status items v1 JSON or unsupported fields"))?;
        if document.version != 1 || document.items.len() > 2 {
            return Err(invalid("unsupported status items version or item count"));
        }
        let mut items = BTreeMap::new();
        for mut item in document.items {
            item.id = item
                .id
                .parse::<StatusLineItem>()
                .ok()
                .filter(|id| is_custom(*id))
                .ok_or_else(|| invalid("unsupported static status item ID"))?
                .to_string();
            if !matches!(item.id.as_str(), "cutex_profile" | "cutex_welcome")
                || item.text.is_empty()
                || item.text.len() > 256
            {
                return Err(invalid(
                    "unsupported status item ID or text length (1..256 bytes)",
                ));
            }
            let text = item.text.chars().map(|c| {
                if c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                    ' '
                } else {
                    c
                }
            }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                return Err(invalid("status item text is empty after sanitation"));
            }
            let style = item.style.resolve()?;
            let animation = item.animation.and_then(|value| {
                match crate::status_animation::Animation::parse(value, style) {
                    Ok(animation) => Some(animation),
                    Err(error) => {
                        tracing::warn!(%error, "Invalid status animation; using static fallback");
                        None
                    }
                }
            });
            if items.insert(item.id, (text, style, animation)).is_some() {
                return Err(invalid("duplicate status item ID"));
            }
        }
        if document.model_display_names.len() > 128 || document.model_display_names.iter().any(|(id, name)| {
            [id, name].iter().any(|s| s.is_empty() || s.len() > 256 || s.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')))
        }) {
            return Err(invalid("invalid model display mapping"));
        }
        Ok(Self(items, document.model_display_names))
    }

    fn value(&self, item: StatusLineItem) -> String {
        if item == StatusLineItem::CutexAlignedProfile {
            let width = unicode_width::UnicodeWidthStr::width(
                self.value(StatusLineItem::CustomBonVoyage).as_str(),
            );
            return centered_profile(&self.value(StatusLineItem::CustomProfile), width);
        }
        self.0.get(&item.to_string()).map_or_else(
            || format!("[{item} unavailable]"),
            |(text, _, _)| text.clone(),
        )
    }

    fn style(&self, item: StatusLineItem) -> Option<Style> {
        let item = if item == StatusLineItem::CutexAlignedProfile {
            StatusLineItem::CustomProfile
        } else {
            item
        };
        self.0.get(&item.to_string()).map(|(_, style, _)| *style)
    }
}

#[allow(clippy::disallowed_methods)] // Explicit reviewed #RRGGBB preserves legacy display style.
pub(crate) fn parse_color(value: &str) -> io::Result<Color> {
    let hex = value
        .strip_prefix('#')
        .filter(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("status item color must be #RRGGBB"))?;
    let rgb = u32::from_str_radix(hex, 16).map_err(|_| invalid("invalid status item color"))?;
    Ok(Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "custom_status_items_tests.rs"]
mod tests;

impl ItemStyle {
    pub(crate) fn resolve(&self) -> io::Result<Style> {
        let mut style = Style::default();
        if let Some(fg) = &self.fg {
            style = style.fg(parse_color(fg)?);
        }
        if let Some(bg) = &self.bg {
            style = style.bg(parse_color(bg)?);
        }
        for (enabled, modifier) in [
            (self.bold, Modifier::BOLD),
            (self.dim, Modifier::DIM),
            (self.italic, Modifier::ITALIC),
            (self.underlined, Modifier::UNDERLINED),
        ] {
            if enabled {
                style = style.add_modifier(modifier);
            }
        }
        Ok(style)
    }
}

/// Expand the existing one-span-per-item line after notification styling has run.
pub(crate) fn animate_line(
    line: &mut ratatui::text::Line<'static>,
    ids: &[StatusLineItem],
) -> Option<std::time::Duration> {
    let empty = StatusItems::default();
    let items = ITEMS.get().map(|(_, items)| items).unwrap_or(&empty);
    let elapsed = ORIGIN
        .get()
        .map(std::time::Instant::elapsed)
        .unwrap_or_default();
    animate_line_with_items(line, ids, items, elapsed)
}
fn animate_line_with_items(
    line: &mut ratatui::text::Line<'static>,
    ids: &[StatusLineItem],
    items: &StatusItems,
    elapsed: std::time::Duration,
) -> Option<std::time::Duration> {
    let mut next = None;
    let mut spans = Vec::new();
    for (index, span) in std::mem::take(&mut line.spans).into_iter().enumerate() {
        let indentation =
            index % 2 == 0 && ids.get(index / 2) == Some(&StatusLineItem::CutexWelcomeIndentation);
        let aligned =
            index % 2 == 0 && ids.get(index / 2) == Some(&StatusLineItem::CutexAlignedProfile);
        let animation = (index % 2 == 0)
            .then(|| ids.get(index / 2))
            .flatten()
            .and_then(|id| {
                items.0.get(
                    &if indentation || aligned {
                        StatusLineItem::CustomBonVoyage
                    } else {
                        *id
                    }
                    .to_string(),
                )
            })
            .and_then(|(_, _, animation)| animation.as_ref());
        if let Some(animation) = animation {
            let (frame, delay) = animation.sample(elapsed);
            if aligned {
                spans.push(ratatui::text::Span::styled(
                    centered_profile(
                        &items.value(StatusLineItem::CustomProfile),
                        frame.iter().map(ratatui::text::Span::width).sum(),
                    ),
                    span.style,
                ));
                if let Some(delay) = delay {
                    next = Some(next.map_or(delay, |old: std::time::Duration| old.min(delay)));
                }
                continue;
            }
            if indentation {
                spans.push(ratatui::text::Span::raw(
                    " ".repeat(frame.iter().map(ratatui::text::Span::width).sum()),
                ));
                continue;
            }
            spans.extend_from_slice(frame);
            if let Some(delay) = delay {
                next = Some(next.map_or(delay, |old: std::time::Duration| old.min(delay)));
            }
        } else if index % 2 == 0
            && let Some(id) = ids.get(index / 2)
        {
            spans.extend(crate::cutex_status_presentation::decorate_span(*id, span));
        } else {
            spans.push(span);
        }
    }
    line.spans = spans;
    next
}
pub(crate) fn has_animation() -> bool {
    ITEMS.get().is_some_and(|(_, items)| {
        items
            .0
            .values()
            .any(|(_, _, animation)| animation.is_some())
    })
}

fn centered_profile(text: &str, width: usize) -> String {
    let line = crate::line_truncation::truncate_line_to_width(
        ratatui::text::Line::raw(text.to_owned()),
        width,
    );
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let padding = width.saturating_sub(unicode_width::UnicodeWidthStr::width(text.as_str()));
    format!(
        "{}{}{}",
        " ".repeat(padding / 2),
        text,
        " ".repeat(padding - padding / 2)
    )
}
