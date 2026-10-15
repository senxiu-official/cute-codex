//! A durable display fact, independent of model conversation items.
use super::*;
use crate::markdown::append_markdown;
use codex_protocol::presentation::PresentationAppended;
use codex_protocol::presentation::PresentationFormat;
use codex_protocol::presentation::PresentationReferenceKind;

#[derive(Debug)]
pub(crate) struct PresentationHistoryCell {
    record: PresentationAppended,
}
impl PresentationHistoryCell {
    pub(crate) fn new(record: PresentationAppended) -> Result<Self, &'static str> {
        record.validate()?;
        Ok(Self { record })
    }
    fn render(
        &self,
        width: u16,
        bullet: Option<Span<'static>>,
        markdown: bool,
        references: bool,
    ) -> Vec<Line<'static>> {
        let p = &self.record.presentation;
        let kind = match p.source.kind {
            codex_protocol::external_input::SourceKind::Agent => "agent",
            codex_protocol::external_input::SourceKind::Service => "service",
        };
        let title = if p.title.is_empty() {
            "Display"
        } else {
            &p.title
        };
        let header = if markdown {
            title.to_owned()
        } else {
            format!("{title} · {kind}/{}", p.source.id)
        };
        let mut detail = Vec::new();
        for reference in p.references.iter().filter(|_| references) {
            let kind = match reference.kind {
                PresentationReferenceKind::ExternalInput => "external input",
                PresentationReferenceKind::McpInvocation => "MCP invocation",
            };
            detail.push(format!(
                "Related {kind}: {} (linked supplement)",
                if markdown {
                    super::job_labels::short_id(&reference.id)
                } else {
                    reference.id.clone()
                }
            ));
        }
        if !markdown {
            detail.push(format!(
                "Presentation: {} · origin {} · receipt {}",
                p.id, self.record.origin_thread_id, self.record.receipt_id
            ));
        }
        let indent = bullet.is_some();
        let mut lines = if markdown && bullet.is_some() {
            super::custom_event_style::header(
                &header,
                super::custom_event_style::Entity::Agent,
                None,
                super::custom_event_style::bullet(),
                width,
            )
        } else {
            super::event_presentation::render_event(Some(&header), &[], bullet.clone(), width)
        };
        lines.extend(
            super::event_presentation::render_event(None, &detail, bullet, width)
                .into_iter()
                .map(ratatui::prelude::Stylize::dim),
        );
        if markdown && p.format == PresentationFormat::Markdown {
            let clean = super::messages::sanitize_user_text(p.body.as_str().into());
            let mut body = Vec::new();
            append_markdown(
                &clean,
                Some(usize::from(
                    width.saturating_sub(if indent { 2 } else { 0 }).max(1),
                )),
                None,
                &mut body,
            );
            for mut line in body {
                if indent {
                    line.spans.insert(0, "  ".into());
                }
                lines.push(line);
            }
        } else {
            lines.extend(super::event_presentation::render_event(
                None,
                std::slice::from_ref(&p.body),
                if indent { Some("•".dim()) } else { None },
                width,
            ));
        }
        lines
    }
}
impl PresentationHistoryCell {
    pub(super) fn group_lines(
        &self,
        width: u16,
        matched: Option<&codex_protocol::presentation::PresentationReference>,
    ) -> Vec<Line<'static>> {
        let mut record = self.record.clone();
        record
            .presentation
            .references
            .retain(|reference| Some(reference) != matched);
        // Display clone only: the original digest, receipt, and references remain in raw/transcript.
        Self { record }.render(width, Some("•".dim()), true, true)
    }
}
impl HistoryCell for PresentationHistoryCell {
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.render(width, Some("•".dim()), false, true)
    }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.render(width, Some("•".dim()), true, true)
    }
    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.render(u16::MAX, None, false, true)
    }
}
