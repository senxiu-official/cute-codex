//! Separate persistent transcript context from the composer's transient hint row.
//! Measurement, rendering, and cursor placement share the same reserved status rectangle.
//! Configured status rows retain their order and reserve a row each before hint/help expansion;
//! clipping keeps the first row first. Inline multirow status shares its last row with warnings,
//! and a footer flash replaces that last status row while retaining configured prefix rows.
//! Shortcut help grows above the input, preserving status and close rows below it even when clipped.

use super::*;

use std::borrow::Cow;

impl ChatComposer {
    pub(crate) fn set_status_line(&mut self, status_line: Option<FooterContent>) -> bool {
        self.set_status_line_rows(status_line.into_iter().collect())
    }

    pub(crate) fn set_status_line_enabled(&mut self, enabled: bool) -> bool {
        if self.footer.status_line_enabled == enabled {
            return false;
        }
        self.footer.status_line_enabled = enabled;
        true
    }

    pub(crate) fn set_side_conversation_context_label(&mut self, label: Option<String>) -> bool {
        if self.footer.side_conversation_context_label == label {
            return false;
        }
        self.footer.side_conversation_context_label = label;
        true
    }

    pub(super) fn status_surface_height(
        &self,
        width: u16,
        available: u16,
        options: ComposerRenderOptions<'_>,
    ) -> u16 {
        // Explicit fork rows precede the last row; native wrapping/extension
        // measurement for that final row follows the upstream geometry contract.
        let prefix_height = if !options.unfocused && self.footer.status_line_enabled {
            u16::try_from(self.footer.status_line_rows.len().saturating_sub(1))
                .unwrap_or(u16::MAX)
                .min(available)
        } else {
            0
        };
        let available = available.saturating_sub(prefix_height);
        // The unfocused composer paints only the raw status, as one dim line.
        let (available, status) = if options.unfocused {
            let raw = self
                .status_line_content()
                .filter(|_| self.footer.status_line_enabled);
            (available.min(1), raw.map(Cow::Borrowed))
        } else {
            let status = self.status_surface_content().filter(|_| {
                options.separate_status_line
                    || self.footer.status_line_rows.len() <= 1
                    || !self.footer.flash_visible()
            });
            (available, status)
        };
        let bounds = inset_footer_hint_area(Rect::new(0, 0, width, available)).as_size();
        if bounds.width == 0 {
            return 0;
        }
        if bounds.height == 0 {
            return prefix_height;
        }
        let last_height = match status.as_deref() {
            Some(FooterContent::Extension { content }) => {
                let cache = &self.footer.measured_status_line;
                if let Some((previous, height)) = cache.get()
                    && previous == bounds
                {
                    return prefix_height.saturating_add(height);
                }
                let height = content.measure(bounds).height;
                cache.set(Some((bounds, height)));
                height
            }
            Some(native) if bounds.height > 1 && self.active_status_transition().is_none() => self
                .wrapped_native_status(native, bounds.width)
                .map_or_else(
                    || u16::from(native.width() > 0),
                    |rows| rows.height().min(bounds.height),
                ),
            Some(line) => u16::from(line.width() > 0),
            None => 0,
        };
        prefix_height.saturating_add(last_height)
    }

    pub(super) fn hint_footer_props(&self) -> FooterProps {
        let mut props = self.footer_props();
        if props.status_line_enabled {
            props.status_line_enabled = false;
            props.status_line_value = None;
            props.status_line_prefix.clear();
            props.active_agent_label = None;
        }
        props
    }

    pub(super) fn status_surface_content(&self) -> Option<Cow<'_, FooterContent>> {
        if !self.footer.status_line_enabled {
            return None;
        }
        let status = self.footer.status_line_rows.last();
        if matches!(status, Some(FooterContent::Extension { .. }))
            || self.footer.active_agent_label.is_none()
        {
            return status.map(Cow::Borrowed);
        }
        let mut props = self.footer_props();
        // The viewed agent belongs to native status even during a transient composer hint.
        props.mode = FooterMode::ComposerHasDraft;
        props.is_task_running = false;
        passive_footer_status_line(&props).map(Cow::Owned)
    }

    fn active_status_transition(&self) -> Option<&EffortStatusLineTransition> {
        self.effort_status_line_transition
            .as_ref()
            .filter(|_| self.footer.status_line_rows.len() <= 1)
            .filter(|transition| !transition.is_finished())
    }

    #[cfg(test)]
    pub(super) fn render_status_surface(
        &self,
        area: Rect,
        buf: &mut Buffer,
        options: ComposerRenderOptions<'_>,
    ) {
        self.render_status_surface_with_warning(area, buf, options, None);
    }

    pub(super) fn render_status_surface_with_warning(
        &self,
        area: Rect,
        buf: &mut Buffer,
        options: ComposerRenderOptions<'_>,
        warning: Option<Rect>,
    ) {
        if area.is_empty() {
            return;
        }
        if options.unfocused {
            if let Some(content) = self.status_line_content() {
                crate::bottom_pane::footer::render_unfocused_status(
                    content.clone().into_hyperlink_line().line.to_string(),
                    area,
                    buf,
                );
            }
            return;
        }
        let mut area = area;
        if self.footer.status_line_enabled {
            for row in self
                .footer
                .status_line_rows
                .iter()
                .take(self.footer.status_line_rows.len().saturating_sub(1))
            {
                if area.is_empty() {
                    return;
                }
                row.clone()
                    .render(inset_footer_hint_area(Rect { height: 1, ..area }), buf);
                area.y = area.y.saturating_add(1);
                area.height = area.height.saturating_sub(1);
            }
        }
        let area = warning_notice::before_warning(inset_footer_hint_area(area), warning);
        if area.is_empty() {
            return;
        }
        let line = self.status_surface_content();
        if let Some(transition) = self.active_status_transition() {
            if let Some(frame) = transition.render_line(
                line.map(|line| line.into_owned().into_hyperlink_line()),
                area.width,
            ) {
                FooterContent::HyperlinkLine(frame).render(area, buf);
            }
            if let Some(requester) = &self.frame_requester {
                requester.schedule_frame_in(EFFORT_STATUS_LINE_FRAME_TICK);
            }
        } else if let Some(line) = line {
            if area.height > 1
                && let Some(wrapped) = self.wrapped_native_status(&line, area.width)
            {
                wrapped.render(area, buf);
            } else {
                line.into_owned().render(area, buf);
            }
        }
    }
}

#[cfg(test)]
#[path = "status_surface_tests.rs"]
mod tests;
