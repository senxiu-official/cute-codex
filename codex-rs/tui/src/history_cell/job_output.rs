//! A display window over a validated output page; never changes retained output.
use super::*;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug)]
pub(super) struct OutputPage {
    bytes: Vec<u8>,
    stream: String,
    from: u64,
    next: u64,
    gap: bool,
    truncated: bool,
    page_limited: bool,
}
impl OutputPage {
    pub(super) fn parse(value: &Value, args: &Value) -> Option<Self> {
        if value["jobId"] != args["jobId"] || value["stream"] != args["stream"] {
            return None;
        }
        let stream = value["stream"].as_str()?;
        if !matches!(stream, "stdout" | "stderr") {
            return None;
        }
        let from = value["fromOffset"].as_u64()?;
        let next = value["nextOffset"].as_u64()?;
        next.checked_sub(from)?;
        let bytes = if let Some(text) = value.get("text").and_then(Value::as_str) {
            if value["encoding"] != "utf-8-lossy" {
                return None;
            }
            text.as_bytes().to_vec()
        } else {
            let hex = value["bytesHex"].as_str()?;
            if hex.len() % 2 != 0 || !hex.is_ascii() {
                return None;
            }
            let bytes: Option<Vec<u8>> = hex
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| {
                    let high = char::from(pair[0]).to_digit(16)?;
                    let low = char::from(pair[1]).to_digit(16)?;
                    Some((high * 16 + low) as u8)
                })
                .collect();
            let bytes = bytes?;
            if from.checked_add(bytes.len() as u64)? != next {
                return None;
            }
            bytes
        };
        Some(Self {
            bytes,
            stream: stream.into(),
            from,
            next,
            gap: value["gap"].as_bool()?,
            truncated: value["truncated"].as_bool()?,
            page_limited: value
                .get("pageLimited")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    pub(super) fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut summary = format!("{} · {} B", self.stream, self.next - self.from);
        if self.from > 0 {
            if self.bytes.is_empty() {
                summary.push_str(&format!(" · offset {}", self.from));
            } else {
                summary.push_str(&format!(" · bytes {}–{}", self.from, self.next - 1));
            }
        }
        if self.bytes.is_empty() {
            summary.push_str(" · empty page");
        }
        let mut metadata = vec![summary];
        if self.gap {
            metadata.push("Output gap reported by source".into());
        }
        if self.truncated {
            metadata.push("Output truncated at source".into());
        }
        if self.page_limited {
            metadata.push("Page limited — more output available".into());
        }
        let mut lines =
            super::event_presentation::render_event(None, &metadata, Some("•".dim()), width);
        for line in &mut lines {
            *line = line.clone().dim();
        }
        let Ok(text) = std::str::from_utf8(&self.bytes) else {
            lines.extend(super::event_presentation::render_event(
                None,
                &[format!(
                    "Binary output · {} B · full page in transcript",
                    self.bytes.len()
                )],
                Some("•".dim()),
                width,
            ));
            return lines;
        };
        let mut end = 0;
        let mut content = Vec::new();
        for (index, grapheme) in text.grapheme_indices(true) {
            let candidate_end = index + grapheme.len();
            if candidate_end > 2048 {
                break;
            }
            let clean = super::messages::sanitize_user_text(text[..candidate_end].into());
            let candidate: Vec<Line<'static>> = clean
                .split('\n')
                .flat_map(|part| {
                    textwrap::wrap(
                        part,
                        textwrap::Options::new(usize::from(width).saturating_sub(2).max(1)),
                    )
                    .into_iter()
                    .map(|part| Line::from(format!("  {part}")).dim())
                    .collect::<Vec<_>>()
                })
                .collect();
            if candidate.len() > 6 {
                break;
            }
            end = candidate_end;
            content = candidate;
        }
        let shortened = end < text.len();
        lines.extend(content);
        if shortened {
            lines.extend(super::event_presentation::render_event(
                None,
                &["Preview shortened — full returned page in transcript".into()],
                Some("•".dim()),
                width,
            ));
        }
        lines
    }
}

#[cfg(test)]
#[path = "job_output_tests.rs"]
mod tests;
