//! Adjacent display composition; both independent facts survive transcript and raw views.
use super::*;
use codex_protocol::presentation::PresentationAppended;
use codex_protocol::presentation::PresentationReferenceKind;

type UngroupedCell = (Box<dyn HistoryCell>, &'static str);

#[derive(Debug)]
pub(crate) struct PendingPresentationGroup {
    pub(crate) record: PresentationAppended,
    pub(crate) counterpart_first: bool,
}
impl PendingPresentationGroup {
    pub(crate) fn matches(&self, cell: &dyn HistoryCell) -> bool {
        self.record
            .presentation
            .references
            .iter()
            .any(|reference| match reference.kind {
                PresentationReferenceKind::ExternalInput => cell
                    .as_any()
                    .downcast_ref::<ExternalInputHistoryCell>()
                    .is_some_and(|cell| cell.id == reference.id),
                PresentationReferenceKind::McpInvocation => cell
                    .as_any()
                    .downcast_ref::<McpToolCallCell>()
                    .is_some_and(|cell| {
                        cell.call_id() == reference.id && cell.supports_compact_presentation()
                    }),
            })
    }
    pub(crate) fn combine(
        self,
        counterpart: Box<dyn HistoryCell>,
    ) -> Result<Box<dyn HistoryCell>, UngroupedCell> {
        let matched = self
            .record
            .presentation
            .references
            .iter()
            .find(|reference| match reference.kind {
                PresentationReferenceKind::ExternalInput => counterpart
                    .as_any()
                    .downcast_ref::<ExternalInputHistoryCell>()
                    .is_some_and(|cell| cell.id == reference.id),
                PresentationReferenceKind::McpInvocation => counterpart
                    .as_any()
                    .downcast_ref::<McpToolCallCell>()
                    .is_some_and(|cell| {
                        cell.call_id() == reference.id && cell.supports_compact_presentation()
                    }),
            })
            .cloned();
        let presentation = match PresentationHistoryCell::new(self.record) {
            Ok(cell) => cell,
            Err(error) => return Err((counterpart, error)),
        };
        Ok(Box::new(PresentationGroupCell {
            matched,
            counterpart,
            presentation,
            counterpart_first: self.counterpart_first,
        }))
    }
}
#[derive(Debug)]
struct PresentationGroupCell {
    matched: Option<codex_protocol::presentation::PresentationReference>,
    counterpart: Box<dyn HistoryCell>,
    presentation: PresentationHistoryCell,
    counterpart_first: bool,
}
impl HistoryCell for PresentationGroupCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let counterpart = self.counterpart.display_lines(width);
        let presentation = self.presentation.group_lines(width, self.matched.as_ref());
        let (mut first, mut second) = if self.counterpart_first {
            (counterpart, presentation)
        } else {
            (presentation, counterpart)
        };
        // Only the second verified native cell loses its bullet, never arbitrary body text.
        if let Some(line) = second.first_mut()
            && let Some(span) = line.spans.first_mut()
            && span.content == "•"
        {
            span.content = " ".into();
        }
        first.push(Line::default());
        first.extend(second);
        first
    }
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let counterpart = self.counterpart.transcript_lines(width);
        let presentation = self.presentation.transcript_lines(width);
        let (mut first, second) = if self.counterpart_first {
            (counterpart, presentation)
        } else {
            (presentation, counterpart)
        };
        first.extend(second);
        first
    }
    fn raw_lines(&self) -> Vec<Line<'static>> {
        let counterpart = self.counterpart.raw_lines();
        let presentation = self.presentation.raw_lines();
        let (mut first, second) = if self.counterpart_first {
            (counterpart, presentation)
        } else {
            (presentation, counterpart)
        };
        first.extend(second);
        first
    }
}
