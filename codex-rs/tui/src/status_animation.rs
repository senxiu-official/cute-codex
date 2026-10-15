//! Inert status-line frame playback. No configuration lookup or I/O on a tick.
use std::time::Duration;

use ratatui::style::Style;
use ratatui::text::Span;
use serde::Deserialize;
use unicode_width::UnicodeWidthStr;

use crate::custom_status_items::ItemStyle;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    width: usize,
    #[serde(default)]
    repeat: bool,
    frames: Vec<Frame>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    duration_ms: u64,
    spans: Vec<Fragment>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fragment {
    text: String,
    #[serde(default)]
    style: ItemStyle,
}

pub(crate) struct Animation {
    ends: Vec<u64>,
    frames: Vec<Vec<Span<'static>>>,
    repeat: bool,
}

impl Animation {
    pub(crate) fn parse(value: serde_json::Value, base: Style) -> Result<Self, String> {
        let doc: Document = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if !(1..=256).contains(&doc.width) || doc.frames.is_empty() || doc.frames.len() > 2048 {
            return Err("animation width or frame count out of range".into());
        }
        let mut ends = Vec::new();
        let mut frames = Vec::new();
        let mut elapsed = 0_u64;
        for frame in doc.frames {
            if !(16..=3_600_000).contains(&frame.duration_ms) || frame.spans.len() > 256 {
                return Err("animation duration or span count out of range".into());
            }
            let mut width = 0;
            let mut spans = Vec::new();
            for fragment in frame.spans {
                if fragment.text.len() > 4096 || fragment.text.chars().any(|c| c.is_control() || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
                    return Err("animation text must be plain single-line text".into());
                }
                width += fragment.text.width();
                let style = fragment.style.resolve().map_err(|e| e.to_string())?;
                spans.push(Span::styled(fragment.text, base.patch(style)));
            }
            if width > doc.width {
                return Err("animation frame exceeds reserved display columns".into());
            }
            spans.push(Span::styled(" ".repeat(doc.width - width), base));
            elapsed += frame.duration_ms;
            ends.push(elapsed);
            frames.push(spans);
        }
        Ok(Self {
            ends,
            frames,
            repeat: doc.repeat,
        })
    }

    #[expect(
        clippy::expect_used,
        reason = "parse guarantees nonempty frames and durations; fields are private"
    )]
    pub(crate) fn sample(&self, elapsed: Duration) -> (&[Span<'static>], Option<Duration>) {
        let total = *self.ends.last().expect("validated nonempty animation");
        let elapsed = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        if !self.repeat && elapsed >= total {
            return (self.frames.last().expect("validated animation"), None);
        }
        let position = if self.repeat {
            elapsed % total
        } else {
            elapsed
        };
        let index = self.ends.partition_point(|end| *end <= position);
        (
            &self.frames[index],
            Some(Duration::from_millis(self.ends[index] - position)),
        )
    }
}

#[cfg(test)]
#[path = "status_animation_tests.rs"]
mod tests;
