//! One view for the server's validated canonical external input projection.
use super::*;
use codex_protocol::external_input::Source;
use codex_protocol::external_input::SourceKind;
use codex_protocol::models::FunctionCallOutputBody;
use serde::Deserialize;
#[path = "external_job_view.rs"]
mod job_view;
#[path = "external_task_view.rs"]
mod task_view;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    source: Source,
    #[serde(rename = "type")]
    event_type: String,
    text: String,
}
#[derive(Debug)]
pub(crate) struct ExternalInputHistoryCell {
    pub(super) id: String,
    header: String,
    body: String,
    view: Option<codex_protocol::external_input_view::View>,
    id_label: Option<String>,
    agent_header: Option<String>,
    task_display: Option<(String, Vec<String>, Option<String>)>,
}
impl ExternalInputHistoryCell {
    pub(crate) fn observe_display_id(&mut self, labels: &mut super::JobLabels) {
        if let Some(view) = &self.view
            && job_view::render(view, None).is_some()
            && let Some(id) = view.data["jobId"].as_str()
        {
            self.id_label = Some(labels.observe_display_id(id));
        }
    }
    pub(crate) fn parse(
        id: &str,
        name: &str,
        namespace: Option<&str>,
        output: &FunctionCallOutputBody,
        view: Option<&codex_protocol::external_input_view::View>,
    ) -> Option<Self> {
        if name != "external_event"
            || namespace != Some("external")
            || id.is_empty()
            || id.len() > 256
        {
            return None;
        }
        let FunctionCallOutputBody::Text(text) = output else {
            return None;
        };
        // Maximum UTF-8 body after JSON escaping, plus bounded source/type wrapper.
        if text.len() > 6 * 65536 + 4096 {
            return None;
        }
        let body: Body = serde_json::from_str(text).ok()?;
        if [&body.source.id, &body.event_type]
            .iter()
            .any(|s| s.is_empty() || s.len() > 256)
            || body.text.is_empty()
            || body.text.len() > 65536
        {
            return None;
        }
        let kind = match body.source.kind {
            SourceKind::Agent => "agent",
            SourceKind::Service => "service",
        };
        let agent_header = view
            .filter(|view| view.schema == "cutex.agent-message.v1")
            .filter(|view| {
                body.source.kind == SourceKind::Agent
                    && body.event_type == "message"
                    && view.data["senderId"].as_str() == Some(body.source.id.as_str())
            })
            .and_then(|view| {
                let mode = match view.data["deliveryMode"].as_str()? {
                    "soon" => "soon",
                    "passive" => "passive",
                    "after_turn" => "after-turn",
                    _ => return None,
                };
                let name = view.data["senderName"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&body.source.id);
                Some(format!("Received message from {name} · {mode}"))
            });
        let task_display = task_view::render(&body, view);
        Some(Self {
            id: id.into(),
            header: format!(
                "External input · {kind}/{} · {}",
                body.source.id, body.event_type
            ),
            body: body.text,
            view: view.cloned(),
            id_label: None,
            agent_header,
            task_display,
        })
    }
}
impl HistoryCell for ExternalInputHistoryCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        if let Some(header) = &self.agent_header {
            let bullet = super::custom_event_style::bullet();
            let time = self
                .view
                .as_ref()
                .and_then(|view| view.data["occurredAtEpochSeconds"].as_i64())
                .and_then(|seconds| seconds.checked_mul(1000))
                .and_then(super::custom_event_style::timestamp);
            let mut lines = super::custom_event_style::header(
                header,
                super::custom_event_style::Entity::Agent,
                time,
                bullet.clone(),
                width,
            );
            let text = self
                .body
                .strip_prefix("Message Type: MESSAGE\nPayload:\n")
                .unwrap_or(&self.body);
            let preview = super::event_presentation::preview(
                text,
                2,
                usize::from(width).saturating_sub(2).max(1),
            );
            lines.extend(
                super::event_presentation::render_event(None, &[preview], Some(bullet), width)
                    .into_iter()
                    .map(Stylize::dim),
            );
            return lines;
        }
        if let Some((header, body, time)) = &self.task_display {
            let bullet = super::custom_event_style::bullet();
            let mut lines = super::custom_event_style::header(
                header,
                super::custom_event_style::Entity::Task,
                time.clone(),
                bullet.clone(),
                width,
            );
            lines.extend(
                super::event_presentation::render_event(None, body, Some(bullet), width)
                    .into_iter()
                    .map(Stylize::dim),
            );
            return lines;
        }
        if let Some((header, body)) = self
            .view
            .as_ref()
            .and_then(|view| job_view::render(view, self.id_label.as_deref()))
        {
            let time = self
                .view
                .as_ref()
                .and_then(|view| {
                    view.data
                        .pointer("/execution/exitObservedAtEpochMillis")
                        .and_then(serde_json::Value::as_i64)
                })
                .and_then(super::custom_event_style::timestamp);
            let mut lines = super::custom_event_style::header(
                &header,
                super::custom_event_style::Entity::Job,
                time,
                super::custom_event_style::bullet(),
                width,
            );
            lines.extend(
                super::event_presentation::render_event(None, &body, Some("•".dim()), width)
                    .into_iter()
                    .map(Stylize::dim),
            );
            return lines;
        }
        super::event_presentation::render_event(
            Some(&self.header),
            std::slice::from_ref(&self.body),
            Some("•".dim()),
            width,
        )
    }
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = super::event_presentation::render_event(
            Some(&self.header),
            std::slice::from_ref(&self.body),
            None,
            width,
        );
        if let Some(view) = &self.view {
            lines.extend(super::event_presentation::render_event(
                Some("Display facts (not model input)"),
                &[view.canonical_json()],
                None,
                width,
            ));
        }
        lines
    }
    fn raw_lines(&self) -> Vec<Line<'static>> {
        let mut lines = super::event_presentation::render_event(
            Some(&self.header),
            std::slice::from_ref(&self.body),
            None,
            u16::MAX,
        );
        if let Some(view) = &self.view {
            lines.extend(super::event_presentation::render_event(
                Some("Display facts (not model input)"),
                &[view.canonical_json()],
                None,
                u16::MAX,
            ));
        }
        lines
    }
}

#[cfg(test)]
#[path = "external_input_view_tests.rs"]
mod view_tests;
