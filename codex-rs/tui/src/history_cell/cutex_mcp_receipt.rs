//! Conservative display interpretation of known returned receipts. No effects or authority.
use super::*;

pub(in crate::history_cell) struct Outcome {
    pub(in crate::history_cell) text: String,
    pub(in crate::history_cell) job_id: Option<String>,
    pub(in crate::history_cell) occurred_at_millis: Option<i64>,
    pub(in crate::history_cell) detail: Vec<String>,
    pub(in crate::history_cell) output: Option<super::super::job_output::OutputPage>,
    pub(in crate::history_cell) failed: bool,
    pub(in crate::history_cell) uncertain: bool,
}

pub(in crate::history_cell) fn outcome(invocation: &McpInvocation, body: &str) -> Option<Outcome> {
    let Some(preset) = lookup(invocation) else {
        return super::tools::outcome(invocation, body);
    };
    let value: Value = serde_json::from_str(body).ok()?;
    let args = invocation.arguments.as_ref()?;
    let mut job_id = None;
    let mut detail = Vec::new();
    let mut output = None;
    let mut failed = false;
    let mut uncertain = false;
    let summary = if preset.server == "cutex_job" {
        if preset.tool == "read_output" {
            job_id = Some(value["jobId"].as_str()?.into());
            output = Some(super::super::job_output::OutputPage::parse(&value, args)?);
            format!("Read job output · {}", value["jobId"].as_str()?)
        } else {
            let job = if preset.tool == "submit" {
                if value["status"] != "committed" || !value["deduplicated"].is_boolean() {
                    return None;
                }
                &value["job"]
            } else {
                &value
            };
            if job["schema"] != "cutex/job-service-core/v1" || job["revision"].as_u64()? == 0 {
                return None;
            }
            let id = job["jobId"].as_str()?;
            job_id = Some(id.into());
            if preset.tool == "submit" && job["request"]["actionId"] != args["actionId"] {
                return None;
            }
            if preset.tool != "submit" && job["jobId"] != args["jobId"] {
                return None;
            }
            let state = job["state"].as_str()?;
            if !matches!(
                state,
                "launch_pending"
                    | "running"
                    | "exited"
                    | "failed"
                    | "cancelled"
                    | "interrupted"
                    | "launch_unknown"
            ) {
                return None;
            }
            failed = matches!(state, "failed" | "interrupted")
                || (state == "exited" && job["exitCode"].as_i64().is_some_and(|n| n != 0));
            uncertain = state == "launch_unknown";
            let verb = match preset.tool {
                "submit" => "Submitted job",
                "cancel" => "Job cancellation returned",
                _ => "Queried job",
            };
            detail.push(format!("State: {state}"));
            if let Some(code) = job["exitCode"].as_i64() {
                detail.push(format!("Exit code: {code}"));
            }
            if preset.tool == "submit" {
                format!("{verb} · {} · {id}", args["actionId"].as_str()?)
            } else {
                format!("{verb} · {id}")
            }
        }
    } else if preset.tool == "send" {
        if value["http_status"]
            .as_u64()
            .is_some_and(|status| (400..=599).contains(&status))
            && let Some(message) = value["error"]["message"].as_str()
        {
            return Some(Outcome {
                text: format!(
                    "Message request failed to {} · {}",
                    target(invocation, preset),
                    args["delivery_mode"].as_str()?.replace('_', "-")
                ),
                job_id: None,
                occurred_at_millis: None,
                detail: vec![
                    super::super::messages::sanitize_user_text(message.into()).into_owned(),
                ],
                output: None,
                failed: true,
                uncertain: false,
            });
        }
        let matches_target = value["to_cutex_session_id"] == args["to"]
            || value["to"] == args["to"]
            || value["to_name"] == args["to"];
        if value["ok"] != true || !matches_target || value["id"].as_str().is_none() {
            return None;
        }
        if value["queued"] != true {
            return None;
        }
        let name = value["to_name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(|name| super::super::messages::sanitize_user_text(name.into()))
            .map(|name| {
                name.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .graphemes(true)
                    .take(80)
                    .collect::<String>()
            })
            .unwrap_or_else(|| target(invocation, preset));
        let mode = args["delivery_mode"].as_str()?.replace('_', "-");
        detail.push(args["message"].as_str()?.into());
        format!("Sent message to {name} · {mode}")
    } else if preset.tool == "cutex_agent_list" {
        if value["ok"] != true || value["scope"] != "local_group_visible" {
            return None;
        }
        format!(
            "Listed agents · {} visible",
            value["agents"].as_array()?.len()
        )
    } else if preset.tool == "cutex_task_service_read" {
        if value["schema"] != "cutex/task-service-query-response/v2" {
            return None;
        }
        let outcome = &value["outcome"];
        if outcome["kind"] == "no_write" {
            failed = true;
            let body = &outcome["body"];
            let code = body["code"].as_str()?;
            if code.is_empty() || code.chars().any(char::is_control) {
                return None;
            }
            detail.push(format!("Code: {code}"));
            format!("Task contract not read · {}", target(invocation, preset))
        } else {
            if outcome["kind"] != "contract" {
                return None;
            }
            let body = &outcome["body"];
            let revision = body["task_revision"].as_u64().filter(|n| *n > 0)?;
            let task_id = body["task_id"].as_str()?;
            let contract_sha256 = body["contract_sha256"].as_str()?;
            if task_id.trim().is_empty()
                || task_id.chars().any(char::is_control)
                || contract_sha256.len() != 64
                || !contract_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return None;
            }
            detail.push(format!("Task: {task_id}"));
            detail.push(format!("Revision: {revision}"));
            detail.push(format!("Contract SHA-256: {contract_sha256}"));
            format!("Read task contract · {}", target(invocation, preset))
        }
    } else if matches!(preset.tool, "cutex_agent_management" | "query_managed") {
        if value["schema"] != "cutex/agent-management/v1" || value["action_id"] != args["action_id"]
        {
            return None;
        }
        let result = &value["outcome"];
        match result["status"].as_str()? {
            "no_write" => {
                uncertain = result["code"] == "response_uncertain";
                failed = !uncertain;
                format!(
                    "{} · {}",
                    if uncertain {
                        "Outcome uncertain"
                    } else {
                        "No successful receipt"
                    },
                    target(invocation, preset)
                )
            }
            "owner_action_required" => {
                failed = true;
                format!("Owner action required · {}", target(invocation, preset))
            }
            "complete" => {
                let receipt = &result["receipt"];
                let op = if preset.operation.is_empty() {
                    "query_managed"
                } else {
                    preset.operation
                };
                if receipt["schema"] != "cutex/agent-management-receipt/v1"
                    || receipt["action_id"] != args["action_id"]
                    || receipt["operation"] != op
                {
                    return None;
                }
                let (kind, verb) = match op {
                    "create" => ("created", "Created agent"),
                    "query_managed" => ("query_managed", "Queried managed agents"),
                    "replace" => ("replaced", "Replaced agent"),
                    "director_rotate" => ("director_rotated", "Rotated director"),
                    "online" => ("lifecycle", "Agent online action recorded"),
                    "offline" => ("lifecycle", "Agent offline action recorded"),
                    "restart" => ("lifecycle", "Agent restart recorded"),
                    "close" => ("lifecycle", "Agent close recorded"),
                    _ => return None,
                };
                if receipt["result"]["kind"] != kind {
                    return None;
                }
                format!("{verb} · {}", target(invocation, preset))
            }
            _ => return None,
        }
    } else {
        let schema = if preset.tool == "cutex_task_service" {
            "cutex/task-service-tool-receipt/v1"
        } else if preset.tool == "cutex_task_service_terminal" {
            "cutex/task-service-terminal-tool-receipt/v1"
        } else {
            "cutex/task-service-director-tool-receipt/v1"
        };
        if value["schema"] != schema
            || value
                .get("action_id")
                .is_some_and(|id| id != &args["action_id"])
        {
            return None;
        }
        let status = value["status"].as_str()?;
        let label = match status {
            "response_uncertain" => {
                uncertain = true;
                "Outcome uncertain"
            }
            "no_write" | "conflict" => {
                failed = true;
                "Task action not confirmed"
            }
            "current_state" => "Task state returned",
            "committed" => match preset.operation {
                "start" => "Started task",
                "report_status" => "Reported task status",
                "block" => "Blocked task",
                "resume" => "Resumed task",
                "submit" => "Submitted task result",
                "decline" => "Declined task",
                "abort_attempt" => "Aborted task attempt",
                "create_revision" => "Created task revision",
                "assign" => "Assigned task",
                "create_and_assign" => "Created and assigned task",
                "query" => "Queried tasks",
                "accept_result" => "Accepted task result",
                "request_changes" => "Requested task changes",
                "fail_result" => "Rejected task result",
                "cancel" => "Cancelled task",
                _ => return None,
            },
            _ => return None,
        };
        format!("{label} · {}", target(invocation, preset))
    };
    Some(Outcome {
        text: summary.trim_end_matches(" · ").into(),
        job_id,
        occurred_at_millis: if preset.tool == "send" {
            value["created_at_epoch_secs"]
                .as_i64()
                .filter(|seconds| *seconds > 0)
                .and_then(|seconds| seconds.checked_mul(1000))
        } else {
            None
        },
        detail,
        output,
        failed,
        uncertain,
    })
}
