//! Same-thread display labels learned only from validated submit receipts.
use super::cutex_mcp_display;
use super::mcp::McpInvocation;
use codex_app_server_protocol::ThreadItem;
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub(crate) struct JobLabels(
    BTreeMap<String, Option<String>>,
    std::collections::BTreeSet<String>,
);

pub(super) fn short_id(id: &str) -> String {
    let chars: Vec<_> = id.chars().collect();
    if chars.len() <= 20 {
        return id.into();
    }
    format!(
        "{}…{}",
        chars[..12].iter().collect::<String>(),
        chars[chars.len() - 4..].iter().collect::<String>()
    )
}
impl JobLabels {
    pub(crate) fn observe(&mut self, item: &ThreadItem) -> Option<String> {
        let ThreadItem::McpToolCall {
            server,
            tool,
            arguments,
            result: Some(result),
            error: None,
            status,
            ..
        } = item
        else {
            return None;
        };
        if server != "cutex_job"
            || tool != "submit"
            || *status != codex_app_server_protocol::McpToolCallStatus::Completed
            || result.content.len() != 1
        {
            return None;
        }
        let block = &result.content[0];
        if block["type"] != "text" {
            return None;
        }
        let body = block["text"].as_str()?;
        if result
            .structured_content
            .as_ref()
            .is_some_and(|structured| {
                serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .as_ref()
                    != Some(structured)
            })
        {
            return None;
        }
        let invocation = McpInvocation {
            server: server.clone(),
            tool: tool.clone(),
            arguments: Some(arguments.clone()),
        };
        let outcome = cutex_mcp_display::outcome(&invocation, body)?;
        let id = outcome.job_id?;
        let action = arguments["actionId"].as_str()?;
        self.0
            .entry(id.clone())
            .and_modify(|previous| {
                if previous.as_deref() != Some(action) {
                    *previous = None;
                }
            })
            .or_insert_with(|| Some(action.into()));
        Some(self.id_label(&id))
    }
    pub(crate) fn observe_display_id(&mut self, id: &str) -> String {
        self.1.insert(id.into());
        self.id_label(id)
    }
    fn id_label(&self, id: &str) -> String {
        let short = short_id(id);
        if self
            .0
            .keys()
            .chain(self.1.iter())
            .any(|other| other != id && short_id(other) == short)
        {
            id.to_owned()
        } else {
            short
        }
    }
    pub(crate) fn label(&self, id: &str) -> String {
        let display = self.id_label(id);
        match self.0.get(id).and_then(Option::as_ref) {
            Some(action) => format!("{action} · {display}"),
            None => display,
        }
    }
}

#[cfg(test)]
#[path = "job_labels_tests.rs"]
mod tests;
