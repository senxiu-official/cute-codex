//! Separate persistent transcript context from the composer's transient hint row.
//! Measurement, rendering, and cursor placement share the same reserved status rectangle.
//! Configured status rows retain their order; clipping keeps the first row first.
//! Shortcut help grows above the input, preserving status and close rows below it even when clipped.

use super::*;

impl ChatComposer {
    pub(super) fn status_surface_height(&self, options: ComposerRenderOptions<'_>) -> u16 {
        if options.separate_status_line && self.footer.status_line_enabled {
            self.footer.status_line_rows.len().max(1) as u16
        } else {
            0
        }
    }

    pub(super) fn hint_footer_props(&self, options: ComposerRenderOptions<'_>) -> FooterProps {
        let mut props = self.footer_props();
        if self.status_surface_height(options) > 0 {
            props.status_line_enabled = false;
            props.status_line_value = None;
            props.status_line_prefix.clear();
            props.active_agent_label = None;
        }
        props
    }

    pub(super) fn render_status_surface(
        &self,
        area: Rect,
        buf: &mut Buffer,
        options: ComposerRenderOptions<'_>,
    ) {
        if area.is_empty() {
            return;
        }
        let mut props = self.footer_props();
        let mut area = area;
        for content in std::mem::take(&mut props.status_line_prefix) {
            if area.is_empty() {
                return;
            }
            let row = Rect::new(area.x, area.y, area.width, 1);
            content.render(inset_footer_hint_area(row), buf);
            area.y += 1;
            area.height -= 1;
        }
        if area.is_empty() {
            return;
        }
        let show_cycle_hint = !props.is_task_running
            && !self.blocks_direct_input
            && self.draft.input_enabled
            && !self.popup_active()
            && matches!(
                props.mode,
                FooterMode::ComposerEmpty | FooterMode::ComposerHasDraft
            )
            && options.footer.is_none_or(|footer| !footer.is_interactive);
        // Persistent context is independent of help, queue, search, and quit modes.
        props.mode = FooterMode::ComposerHasDraft;
        props.is_task_running = false;
        let line = passive_footer_status_line(&props);
        let full = self.mode_indicator_line(show_cycle_hint);
        let left_width = line.as_ref().map_or(/*default*/ 0, FooterContent::width);
        let full_width = full
            .as_ref()
            .map_or(/*default*/ 0, |line| line.width() as u16);
        let right = if full_width <= inset_footer_hint_area(area).width
            && can_show_left_with_context(area, left_width, full_width)
        {
            full
        } else {
            self.mode_indicator_line(/*show_cycle_hint*/ false)
        };
        let right_width = right
            .as_ref()
            .map_or(/*default*/ 0, |line| line.width() as u16);
        let width = max_left_width_for_right(area, right_width)
            .unwrap_or_else(|| inset_footer_hint_area(area).width);
        let transition = self
            .effort_status_line_transition
            .as_ref()
            .filter(|transition| !transition.is_finished());
        let content = if let Some(transition) = transition {
            let current = line.map(FooterContent::into_hyperlink_line);
            transition
                .render_line(current, width)
                .map(FooterContent::HyperlinkLine)
        } else {
            line
        };
        if let Some(content) = content {
            content.render(
                Rect {
                    width,
                    ..inset_footer_hint_area(area)
                },
                buf,
            );
        }
        if let Some(right) = right {
            render_context_right(area, buf, &right);
        }
        if transition.is_some()
            && let Some(frame_requester) = &self.frame_requester
        {
            frame_requester.schedule_frame_in(EFFORT_STATUS_LINE_FRAME_TICK);
        }
    }
}

#[cfg(test)]
#[path = "status_surface_tests.rs"]
mod tests;
