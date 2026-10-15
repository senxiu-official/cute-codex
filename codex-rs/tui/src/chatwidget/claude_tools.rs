//! Display SDK-owned native Claude tools without treating them as executable TUI requests.
use super::*;

impl ChatWidget {
    #[expect(
        clippy::disallowed_methods,
        reason = "preserve the configured Cutex Claude tool accent"
    )]
    pub(super) fn on_claude_tool(&mut self, item: ThreadItem) {
        let ThreadItem::DynamicToolCall {
            id,
            namespace,
            tool,
            arguments,
            status,
            success,
            ..
        } = item
        else {
            return;
        };
        if namespace.as_deref() != Some("claude") || tool == "AskUserQuestion" {
            return;
        }
        let running = status == codex_app_server_protocol::DynamicToolCallStatus::InProgress;
        let detail = ["query", "url", "file_path", "command", "pattern"]
            .iter()
            .find_map(|key| arguments.get(key).and_then(serde_json::Value::as_str))
            .unwrap_or("");
        let detail = detail.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut preview: String = detail.chars().take(180).collect();
        if detail.chars().count() > 180 {
            preview.push('…');
        }
        if tool == "WebSearch" {
            if running {
                self.on_web_search_begin(id);
            } else {
                if success == Some(false) {
                    preview.push_str(" (failed)");
                }
                self.on_web_search_end(
                    id,
                    preview,
                    codex_app_server_protocol::WebSearchAction::Other,
                );
            }
            return;
        }
        if running {
            return;
        }
        self.flush_answer_stream_with_separator();
        self.flush_active_cell();
        let outcome = if success == Some(false) {
            " · failed"
        } else {
            ""
        };
        let text = if preview.is_empty() {
            format!("Claude {tool}{outcome}")
        } else {
            format!("Claude {tool}{outcome}\n{preview}")
        };
        self.add_to_history(history_cell::PrefixedWrappedHistoryCell::new(
            text,
            "• ".fg(ratatui::style::Color::Rgb(224, 142, 178)),
            "  ",
        ));
    }
}
