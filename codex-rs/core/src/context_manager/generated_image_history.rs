//! cute-codex: put generated images away from the model request in batches.
//!
//! An image tool output keeps its image inline in history, so every request re-sends every
//! generated image. When `generated_image_history_max_inline` is configured, the turn start
//! check puts away every generated image except those among the newest `keep` images once more
//! than `max_inline` generated images are still inline. A put-away image is sent as a short text
//! placeholder naming its saved file. Between two batches the request prefix does not change, so
//! the prompt cache breaks once per batch rather than once per turn.
//!
//! Raw history items are never modified: rollouts, resume and image editing keep the bytes. The
//! put-away set lives on the in-memory `ContextManager` only, so a resumed thread starts with an
//! empty set and its first turn re-evaluates (at most one extra cache break).
//!
//! An image counts as generated with a saved file only when the content item right after it is
//! the image tool's hint text ("Generated images are saved to <dir> as <path> by default.").
//! Other images (view_image results, user attachments, unsaved images) are never put away, but
//! they still count toward the newest `keep` images.

use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseItem;
use std::collections::HashSet;

const SAVED_IMAGE_HINT_PREFIX: &str = "Generated images are saved to ";
const SAVED_IMAGE_HINT_SUFFIX: &str = " by default.";

/// Identifies one image inside a tool output: (call id, content index).
pub(super) type PutAwayImageKey = (String, usize);

/// One image of the history, oldest first.
struct ImageSlot<'a> {
    /// Set only for generated images with a parseable saved path.
    candidate: Option<(PutAwayImageKey, &'a str)>,
}

/// Returns the saved path from an image tool hint, if `text` is one.
pub(super) fn parse_saved_image_path(text: &str) -> Option<&str> {
    let first_line = text.lines().next()?;
    let rest = first_line
        .strip_prefix(SAVED_IMAGE_HINT_PREFIX)?
        .strip_suffix(SAVED_IMAGE_HINT_SUFFIX)?;
    // "<dir> as <path>"; either part may contain " as ", so prefer the split where the path
    // lies under the directory and fall back to the last separator.
    let splits: Vec<usize> = rest.match_indices(" as ").map(|(index, _)| index).collect();
    let split = splits
        .iter()
        .copied()
        .find(|&index| {
            let dir = &rest[..index];
            let path = &rest[index + 4..];
            !dir.is_empty() && path.starts_with(dir)
        })
        .or_else(|| splits.last().copied())?;
    let path = rest[split + 4..].trim();
    (!path.is_empty()).then_some(path)
}

/// Model-visible text that replaces a put-away image.
pub(super) fn put_away_image_placeholder(path: &str) -> String {
    format!(
        "[Image removed from context to save tokens; saved at {path}. To see it again use view_image on that path; to edit it pass the path in referenced_image_paths.]"
    )
}

fn tool_output_parts(item: &ResponseItem) -> Option<(&str, &[FunctionCallOutputContentItem])> {
    let (call_id, output) = match item {
        ResponseItem::FunctionCallOutput {
            call_id: Some(call_id),
            output,
            ..
        }
        | ResponseItem::CustomToolCallOutput {
            call_id, output, ..
        } => (call_id.as_str(), output),
        _ => return None,
    };
    match &output.body {
        FunctionCallOutputBody::ContentItems(parts) => Some((call_id, parts)),
        FunctionCallOutputBody::Text(_) => None,
    }
}

/// Saved path of the generated image at `index`, if the next part is the image tool's hint.
fn saved_path_after(parts: &[FunctionCallOutputContentItem], index: usize) -> Option<&str> {
    match parts.get(index + 1) {
        Some(FunctionCallOutputContentItem::InputText { text }) => parse_saved_image_path(text),
        _ => None,
    }
}

fn image_slots<'a>(items: impl Iterator<Item = &'a ResponseItem>) -> Vec<ImageSlot<'a>> {
    let mut slots = Vec::new();
    for item in items {
        if let Some((call_id, parts)) = tool_output_parts(item) {
            for (index, part) in parts.iter().enumerate() {
                if matches!(part, FunctionCallOutputContentItem::InputImage { .. }) {
                    slots.push(ImageSlot {
                        candidate: saved_path_after(parts, index)
                            .map(|path| ((call_id.to_string(), index), path)),
                    });
                }
            }
        } else if let ResponseItem::Message { content, .. } = item {
            slots.extend(
                content
                    .iter()
                    .filter(|part| matches!(part, ContentItem::InputImage { .. }))
                    .map(|_| ImageSlot { candidate: None }),
            );
        }
    }
    slots
}

/// Applies the batch rule and returns how many images were newly put away.
pub(super) fn put_away<'a>(
    items: impl Iterator<Item = &'a ResponseItem>,
    put_away: &mut HashSet<PutAwayImageKey>,
    max_inline: usize,
    keep: usize,
) -> usize {
    let slots = image_slots(items);
    let inline = slots
        .iter()
        .filter_map(|slot| slot.candidate.as_ref())
        .filter(|(key, _)| !put_away.contains(key))
        .count();
    if inline <= max_inline {
        return 0;
    }
    // Forget images that left the history (compaction, rollback).
    let present: HashSet<&PutAwayImageKey> = slots
        .iter()
        .filter_map(|slot| slot.candidate.as_ref().map(|(key, _)| key))
        .collect();
    put_away.retain(|key| present.contains(key));
    let newest_start = slots.len().saturating_sub(keep);
    let mut added = 0;
    for slot in &slots[..newest_start] {
        if let Some((key, _)) = &slot.candidate
            && put_away.insert(key.clone())
        {
            added += 1;
        }
    }
    added
}

/// Put-away images of `item`: (content index, saved path, image part).
pub(super) fn put_away_parts<'a>(
    item: &'a ResponseItem,
    put_away: &HashSet<PutAwayImageKey>,
) -> Vec<(usize, &'a str, &'a FunctionCallOutputContentItem)> {
    let Some((call_id, parts)) = tool_output_parts(item) else {
        return Vec::new();
    };
    parts
        .iter()
        .enumerate()
        .filter(|(index, part)| {
            matches!(part, FunctionCallOutputContentItem::InputImage { .. })
                && put_away.contains(&(call_id.to_string(), *index))
        })
        .filter_map(|(index, part)| saved_path_after(parts, index).map(|path| (index, path, part)))
        .collect()
}

/// Replaces put-away images in `item` with their placeholder text.
pub(super) fn apply_placeholders(item: &mut ResponseItem, put_away: &HashSet<PutAwayImageKey>) {
    let replacements: Vec<(usize, String)> = put_away_parts(item, put_away)
        .into_iter()
        .map(|(index, path, _)| (index, put_away_image_placeholder(path)))
        .collect();
    if replacements.is_empty() {
        return;
    }
    let (ResponseItem::FunctionCallOutput { output, .. }
    | ResponseItem::CustomToolCallOutput { output, .. }) = item
    else {
        return;
    };
    let FunctionCallOutputBody::ContentItems(parts) = &mut output.body else {
        return;
    };
    for (index, text) in replacements {
        parts[index] = FunctionCallOutputContentItem::InputText { text };
    }
}

#[cfg(test)]
#[path = "generated_image_history_tests.rs"]
mod tests;
