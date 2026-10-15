//! Task transition display, with compatibility for already committed legacy text.
use super::*;

pub(super) fn render(
    body: &Body,
    view: Option<&codex_protocol::external_input_view::View>,
) -> Option<(String, Vec<String>, Option<String>)> {
    if body.source.kind != SourceKind::Service
        || body.source.id != "cutex-task-service"
        || body.event_type != "task_notification"
    {
        return None;
    }
    if let Some(first) = body.text.lines().next()
        && let Some(watchdog) = parse_legacy_watchdog(first)
    {
        return Some((
            format!("Task watchdog · {}", watchdog.assignment),
            vec![
                format!(
                    "Attempt {} · no authoritative progress for at least {} seconds",
                    watchdog.attempt, watchdog.seconds
                ),
                watchdog.instruction,
            ],
            None,
        ));
    }
    let (kind, task, assignment, revision, attempt, time) = if let Some(view) =
        view.filter(|v| v.schema == "cutex.task-notification.v1" && v.validate().is_ok())
    {
        let d = &view.data;
        let time = d["occurredAt"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .and_then(|t| super::super::custom_event_style::timestamp(t.timestamp_millis()));
        (
            d["kind"].as_str()?.to_owned(),
            d["taskId"].as_str()?.to_owned(),
            d["assignmentId"].as_str()?.to_owned(),
            d["taskRevision"].as_u64()?.to_string(),
            d["attemptNumber"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".into()),
            time,
        )
    } else {
        let first = body.text.lines().next()?;
        let rest = first.strip_prefix("Task Service transition ")?;
        let (kind, rest) = rest.split_once(" for assignment ")?;
        let (assignment, rest) = rest.split_once(" (task ")?;
        let (task, rest) = rest.rsplit_once(" revision ")?;
        let (revision, attempt) = rest.strip_suffix(").")?.split_once(", attempt ")?;
        if revision.parse::<u64>().is_err()
            || (attempt != "none" && attempt.parse::<u64>().is_err())
        {
            return None;
        }
        (
            kind.into(),
            task.into(),
            assignment.into(),
            revision.into(),
            attempt.into(),
            None,
        )
    };
    if task.is_empty() || assignment.is_empty() || task.len() > 256 || assignment.len() > 256 {
        return None;
    }
    let title = match kind.as_str() {
        "ReviewReady" | "review_ready" => "Task ready for review",
        "TerminalClosure" | "terminal_closure" => "Task closed",
        "Blocked" | "blocked" => "Task blocked",
        "Declined" | "declined" => "Task declined",
        "AttemptAborted" | "attempt_aborted" => "Task attempt aborted",
        "RetriesExhausted" | "retries_exhausted" => "Task retries exhausted",
        "OwnerActionRequired" | "owner_action_required" => "Task needs owner action",
        _ => return None,
    };
    Some((
        format!("{title} · {task}"),
        vec![
            format!("Assignment {assignment} · Revision {revision} · Attempt {attempt}"),
            "Historical transition; query the task for its current state.".into(),
        ],
        time,
    ))
}

struct LegacyWatchdog {
    assignment: String,
    attempt: u64,
    seconds: u64,
    instruction: String,
}

fn parse_legacy_watchdog(first: &str) -> Option<LegacyWatchdog> {
    let rest = first.strip_prefix("Task Service watchdog: assignment ")?;
    let (assignment, rest) = rest.split_once(" attempt ")?;
    let (attempt, rest) = rest.split_once(" has no authoritative progress for at least ")?;
    let (seconds, instruction) = rest.split_once(" seconds. ")?;
    let attempt = attempt.parse::<u64>().ok()?;
    let seconds = seconds.parse::<u64>().ok()?;
    if assignment.is_empty()
        || assignment.len() > 256
        || instruction.is_empty()
        || instruction.len() > 4096
    {
        return None;
    }
    Some(LegacyWatchdog {
        assignment: assignment.to_owned(),
        attempt,
        seconds,
        instruction: instruction.to_owned(),
    })
}
