//! Display-only quota data supplied by Cutex; no credentials or HTTP in the TUI.
use crate::bottom_pane::StatusLineItem;

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Display {
    label: String,
    remaining_percent: Option<f64>,
    reset_at: Option<i64>,
    pub(crate) stale: bool,
}

pub(crate) fn parse_response(bytes: &[u8]) -> Option<Display> {
    #[derive(serde::Deserialize)]
    struct Response {
        custom_limit: Option<Display>,
    }
    let display = serde_json::from_slice::<Response>(bytes)
        .ok()?
        .custom_limit?;
    if display.label.is_empty()
        || display.label.len() > 6
        || !display.label.bytes().all(|c| c.is_ascii_alphanumeric())
        || display.remaining_percent.is_some_and(|n| !n.is_finite())
    {
        return None;
    }
    Some(display)
}

impl Display {
    pub(crate) fn render(&self, item: StatusLineItem, now: i64) -> String {
        let label = if self.stale {
            format!("{}~", self.label)
        } else {
            self.label.clone()
        };
        let Some(remaining) = self.remaining_percent else {
            return format!("{label} —");
        };
        let width = if item == StatusLineItem::CutexCustomizedLimitBar {
            crate::cutex_status_presentation::bar_width(item)
        } else {
            0
        };
        crate::cutex_status_presentation::format_quota(
            100.0 - remaining.clamp(0.0, 100.0),
            self.reset_at,
            now,
            width,
        )
        .replacen("W", &label, 1)
    }
}

#[cfg(test)]
#[path = "cutex_custom_limit_tests.rs"]
mod tests;
