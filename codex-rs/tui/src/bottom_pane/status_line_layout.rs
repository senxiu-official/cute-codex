//! A single movable boundary, shared by live rendering and picker previews.
use super::StatusLineItem;
pub(crate) fn split(items: impl IntoIterator<Item = StatusLineItem>) -> [Vec<StatusLineItem>; 2] {
    let mut rows = [Vec::new(), Vec::new()];
    let mut row = 0;
    for item in items {
        if item == StatusLineItem::CutexNewline {
            row = 1;
        } else {
            rows[row].push(item);
        }
    }
    rows
}
#[cfg(test)]
#[path = "status_line_layout_tests.rs"]
mod tests;

/// Spacers occupy display columns without introducing normal item separators.
pub(crate) fn is_spacing(item: StatusLineItem) -> bool {
    matches!(
        item,
        StatusLineItem::CutexWelcomeIndentation
            | StatusLineItem::CutexSpace2
            | StatusLineItem::CutexSpace4
            | StatusLineItem::CutexSpace6
            | StatusLineItem::CutexSpace8
            | StatusLineItem::CutexSpace10
    )
}
pub(crate) fn spacing(item: StatusLineItem) -> Option<String> {
    let width = match item {
        StatusLineItem::CutexWelcomeIndentation => {
            let welcome = crate::custom_status_items::value(StatusLineItem::CustomBonVoyage);
            unicode_width::UnicodeWidthStr::width(welcome.as_str())
        }
        StatusLineItem::CutexSpace2 => 2,
        StatusLineItem::CutexSpace4 => 4,
        StatusLineItem::CutexSpace6 => 6,
        StatusLineItem::CutexSpace8 => 8,
        StatusLineItem::CutexSpace10 => 10,
        _ => return None,
    };
    Some(" ".repeat(width))
}
