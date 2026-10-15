//! A presentation of descriptive frozen facts, never permission or lookup authority.
use codex_protocol::external_input_view::View;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Facts {
    job_id: String,
    job_revision: u64,
    terminal_status: String,
    action_id: Option<String>,
    exit_code: Option<i32>,
    terminal_reason: Option<String>,
    output_reference: Option<String>,
    execution: Option<Execution>,
    stdout: Option<Stream>,
    stderr: Option<Stream>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Execution {
    basis: String,
    start_observed_at_epoch_millis: Option<u64>,
    exit_observed_at_epoch_millis: Option<u64>,
    observed_run_duration_millis: Option<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Stream {
    observed_bytes: u64,
    retained_bytes: u64,
    truncated: bool,
}

pub(super) fn render(view: &View, id_label: Option<&str>) -> Option<(String, Vec<String>)> {
    if view.schema != "cutex.job-completion.v1" || view.validate().is_err() {
        return None;
    }
    let facts: Facts = serde_json::from_value(view.data.clone()).ok()?;
    if facts.job_revision == 0
        || facts.job_id.is_empty()
        || facts.job_id.len() > 256
        || facts
            .action_id
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 256)
        || facts
            .output_reference
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 2048)
        || facts
            .terminal_reason
            .as_ref()
            .is_some_and(|value| value.len() > 4096)
    {
        return None;
    }
    let title = match facts.terminal_status.as_str() {
        "exited" if facts.exit_code == Some(0) => "Job completed",
        "exited" => "Job exited",
        "failed" => "Job failed",
        "cancelled" => "Job cancelled",
        "interrupted" => "Job interrupted — result unknown",
        "launch_unknown" => "Job launch unknown",
        _ => return None,
    };
    let mut header = title.to_string();
    if let Some(action) = facts.action_id {
        header.push_str(&format!(" · {action}"));
    }
    header.push_str(&format!(
        " · {}",
        id_label
            .map(str::to_owned)
            .unwrap_or_else(|| super::super::job_labels::short_id(&facts.job_id))
    ));
    let mut details = Vec::new();
    if let Some(code) = facts.exit_code {
        details.push(format!("exit {code}"));
    }
    if let Some(execution) = facts.execution {
        if execution.basis != "runner_release_to_wait_v1" {
            return None;
        }
        // Absolute wall observations remain in raw facts; never subtract them.
        let _ = (
            execution.start_observed_at_epoch_millis,
            execution.exit_observed_at_epoch_millis,
        );
        if let Some(duration) = execution.observed_run_duration_millis {
            details.push(format!("run {}.{:03} s", duration / 1000, duration % 1000));
        }
    }
    for (name, stream) in [("stdout", facts.stdout), ("stderr", facts.stderr)] {
        if let Some(stream) = stream {
            if stream.retained_bytes > stream.observed_bytes {
                return None;
            }
            let mut summary = if stream.observed_bytes == stream.retained_bytes {
                format!("{name} {} B", stream.retained_bytes)
            } else {
                format!(
                    "{name} {} B / {} B",
                    stream.retained_bytes, stream.observed_bytes
                )
            };
            if stream.truncated {
                summary.push_str(" · truncated");
            }
            details.push(summary);
        }
    }
    let mut details = if details.is_empty() {
        Vec::new()
    } else {
        vec![details.join(" · ")]
    };
    if let Some(reason) = facts.terminal_reason {
        details.push(reason);
    }
    Some((header, details))
}

#[cfg(test)]
#[path = "external_job_view_tests.rs"]
mod tests;
