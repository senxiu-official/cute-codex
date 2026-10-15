//! Fixed presentation registry, never an execution/identity registry.
use super::mcp::McpInvocation;
use serde_json::Map;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

struct Preset {
    server: &'static str,
    tool: &'static str,
    operation: &'static str,
    running: &'static str,
    target: &'static str,
    required: &'static str,
    optional: &'static str,
}
macro_rules! preset {
    ($server:literal,$tool:literal,$op:literal,$verb:literal,$target:literal,$required:literal,$optional:literal) => {
        Preset {
            server: $server,
            tool: $tool,
            operation: $op,
            running: $verb,
            target: $target,
            required: $required,
            optional: $optional,
        }
    };
}
const PRESETS: &[Preset] = &[
    preset!(
        "cutex_job",
        "submit",
        "",
        "Submitting job",
        "actionId",
        "actionId argv cwd",
        ""
    ),
    preset!(
        "cutex_job",
        "query",
        "",
        "Querying job",
        "jobId",
        "jobId",
        ""
    ),
    preset!(
        "cutex_job",
        "read_output",
        "",
        "Reading job output",
        "jobId",
        "jobId stream",
        "offset maxBytes"
    ),
    preset!(
        "cutex_job",
        "cancel",
        "",
        "Cancelling job",
        "jobId",
        "jobId expectedRevision",
        ""
    ),
    preset!(
        "cutex",
        "send",
        "",
        "Sending message",
        "to",
        "to message external_message_id delivery_mode",
        ""
    ),
    preset!(
        "cutex",
        "cutex_task_service_read",
        "",
        "Reading task contract",
        "assignment_id",
        "assignment_id",
        ""
    ),
    preset!(
        "cutex",
        "cutex_agent_list",
        "",
        "Listing agents",
        "",
        "",
        "all_groups all_hosts"
    ),
    preset!(
        "cutex",
        "query_managed",
        "",
        "Querying managed agents",
        "",
        "action_id",
        "project_id"
    ),
    preset!(
        "cutex",
        "cutex_agent_management",
        "create",
        "Creating agent",
        "spec.name",
        "operation action_id spec start_mode",
        "project_id frozen_message bootstrap_intent"
    ),
    preset!(
        "cutex",
        "cutex_agent_management",
        "query_managed",
        "Querying managed agents",
        "",
        "operation action_id",
        "project_id"
    ),
    preset!(
        "cutex",
        "cutex_agent_management",
        "online",
        "Bringing agent online",
        "cutex_session_id",
        "operation action_id cutex_session_id",
        "project_id"
    ),
    preset!(
        "cutex",
        "cutex_agent_management",
        "offline",
        "Taking agent offline",
        "cutex_session_id",
        "operation action_id cutex_session_id",
        "project_id"
    ),
    preset!(
        "cutex",
        "cutex_agent_management",
        "restart",
        "Restarting agent",
        "cutex_session_id",
        "operation action_id cutex_session_id",
        "project_id"
    ),
    preset!(
        "cutex",
        "cutex_agent_management",
        "close",
        "Closing agent",
        "cutex_session_id",
        "operation action_id cutex_session_id",
        "project_id"
    ),
    preset!(
        "cutex",
        "cutex_agent_management",
        "replace",
        "Replacing agent",
        "predecessor_cutex_session_id",
        "operation action_id predecessor_cutex_session_id policy successor start_mode",
        "project_id frozen_message bootstrap_intent"
    ),
    preset!(
        "cutex",
        "cutex_agent_management",
        "director_rotate",
        "Rotating director",
        "successor.name",
        "operation action_id expected_predecessor_cutex_session expected_authority_epoch mode successor",
        "project_id frozen_message bootstrap_intent"
    ),
    preset!(
        "cutex",
        "cutex_task_service",
        "start",
        "Starting task",
        "assignment_id",
        "operation action_id assignment_id",
        "summary evidence_sha256 result_sha256 result_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service",
        "report_status",
        "Reporting task status",
        "assignment_id",
        "operation action_id assignment_id summary",
        "summary evidence_sha256 result_sha256 result_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service",
        "block",
        "Blocking task",
        "assignment_id",
        "operation action_id assignment_id summary",
        "summary evidence_sha256 result_sha256 result_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service",
        "resume",
        "Resuming task",
        "assignment_id",
        "operation action_id assignment_id",
        "summary evidence_sha256 result_sha256 result_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service",
        "submit",
        "Submitting task result",
        "assignment_id",
        "operation action_id assignment_id result_sha256 result_reference",
        "summary evidence_sha256 result_sha256 result_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service",
        "decline",
        "Declining task",
        "assignment_id",
        "operation action_id assignment_id",
        "summary evidence_sha256 result_sha256 result_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service",
        "abort_attempt",
        "Aborting task attempt",
        "assignment_id",
        "operation action_id assignment_id",
        "summary evidence_sha256 result_sha256 result_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service_director",
        "create_revision",
        "Creating task revision",
        "task_id",
        "operation action_id project_id workflow_id task_id task_revision opaque_contract completion_policy",
        "project_id workflow_id task_id task_revision opaque_contract completion_policy completion_authority_cutex_session_id assignment_id assignee_cutex_session_id summary decision_reference selector"
    ),
    preset!(
        "cutex",
        "cutex_task_service_director",
        "assign",
        "Assigning task",
        "assignment_id",
        "operation action_id project_id task_id task_revision assignment_id assignee_cutex_session_id summary",
        "project_id workflow_id task_id task_revision opaque_contract completion_policy completion_authority_cutex_session_id assignment_id assignee_cutex_session_id summary decision_reference selector"
    ),
    preset!(
        "cutex",
        "cutex_task_service_director",
        "create_and_assign",
        "Creating and assigning task",
        "assignment_id",
        "operation action_id project_id workflow_id task_id task_revision opaque_contract completion_policy assignment_id assignee_cutex_session_id summary",
        "project_id workflow_id task_id task_revision opaque_contract completion_policy completion_authority_cutex_session_id assignment_id assignee_cutex_session_id summary decision_reference selector"
    ),
    preset!(
        "cutex",
        "cutex_task_service_director",
        "query",
        "Querying tasks",
        "",
        "operation action_id selector",
        "project_id workflow_id task_id task_revision opaque_contract completion_policy completion_authority_cutex_session_id assignment_id assignee_cutex_session_id summary decision_reference selector"
    ),
    preset!(
        "cutex",
        "cutex_task_service_director",
        "accept_result",
        "Accepting task result",
        "assignment_id",
        "operation action_id assignment_id",
        "project_id workflow_id task_id task_revision opaque_contract completion_policy completion_authority_cutex_session_id assignment_id assignee_cutex_session_id summary decision_reference selector"
    ),
    preset!(
        "cutex",
        "cutex_task_service_director",
        "request_changes",
        "Requesting task changes",
        "assignment_id",
        "operation action_id assignment_id decision_reference",
        "project_id workflow_id task_id task_revision opaque_contract completion_policy completion_authority_cutex_session_id assignment_id assignee_cutex_session_id summary decision_reference selector"
    ),
    preset!(
        "cutex",
        "cutex_task_service_director",
        "fail_result",
        "Rejecting task result",
        "assignment_id",
        "operation action_id assignment_id",
        "project_id workflow_id task_id task_revision opaque_contract completion_policy completion_authority_cutex_session_id assignment_id assignee_cutex_session_id summary decision_reference selector"
    ),
    preset!(
        "cutex",
        "cutex_task_service_director",
        "cancel",
        "Cancelling task",
        "assignment_id",
        "operation action_id assignment_id",
        "project_id workflow_id task_id task_revision opaque_contract completion_policy completion_authority_cutex_session_id assignment_id assignee_cutex_session_id summary decision_reference selector"
    ),
    preset!(
        "cutex",
        "cutex_task_service_terminal",
        "accept_result",
        "Accepting task result",
        "assignment_id",
        "operation action_id assignment_id",
        "decision_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service_terminal",
        "request_changes",
        "Requesting task changes",
        "assignment_id",
        "operation action_id assignment_id decision_reference",
        "decision_reference"
    ),
    preset!(
        "cutex",
        "cutex_task_service_terminal",
        "fail_result",
        "Rejecting task result",
        "assignment_id",
        "operation action_id assignment_id",
        "decision_reference"
    ),
];
fn text<'a>(args: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    args.get(key)?.as_str().filter(|s| !s.trim().is_empty())
}
fn valid_field(key: &str, value: &Value) -> bool {
    match key {
        "all_groups" | "all_hosts" => value == &Value::Bool(false),
        "argv" => value.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(Value::is_string)),
        "expectedRevision" | "task_revision" => value.as_u64().is_some_and(|n| n > 0 && n <= 9007199254740991),
        "offset" | "expected_authority_epoch" => value.as_u64().is_some(),
        "maxBytes" => value.as_u64().is_some_and(|n| (1..=1048576).contains(&n)),
        "stream" => matches!(value.as_str(),Some("stdout"|"stderr")),
        "delivery_mode" => matches!(value.as_str(),Some("after_turn"|"soon"|"passive"|"interrupt")),
        "start_mode" => matches!(value.as_str(),Some("bootstrap_only"|"custom_message")),
        "policy" => matches!(value.as_str(),Some("close_before_create"|"close_after_ready"|"keep_old")),
        "mode" => matches!(value.as_str(),Some("close_predecessor_then_create_with_message"|"retain_predecessor_with_message"|"retain_predecessor_bootstrap_only")),
        "completion_policy" => matches!(value.as_str(),Some("director_acceptance"|"release_review")),
        "spec" | "successor" => value.as_object().is_some_and(|o| {
            let fields="name cwd profile runtime_backend model reasoning permissions approval_policy sandbox_mode";
            fields.split_whitespace().all(|k| text(o,k).is_some())
                && o.get("groups").and_then(Value::as_array).is_some_and(|a| a.iter().all(Value::is_string))
                && o.iter().all(|(k,v)| fields.split_whitespace().any(|f|f==k) || k=="groups" || (matches!(k.as_str(),"pin"|"expose_to_im") && v.is_boolean()))
        }),
        "selector" => value.as_object().is_some_and(|o| {
            matches!(text(o,"kind"),Some("all"|"task"|"assignment"))
                && o.iter().all(|(k,v)| matches!(k.as_str(),"kind"|"task_id"|"assignment_id") && v.is_string())
                && (text(o,"kind")!=Some("task") || text(o,"task_id").is_some())
                && (text(o,"kind")!=Some("assignment") || text(o,"assignment_id").is_some())
        }),
        _ => value.is_string(),
    }
}
fn lookup(invocation: &McpInvocation) -> Option<&'static Preset> {
    let args = invocation.arguments.as_ref()?.as_object()?;
    let preset = PRESETS.iter().find(|p| {
        p.server == invocation.server
            && p.tool == invocation.tool
            && (p.operation.is_empty() || text(args, "operation") == Some(p.operation))
    })?;
    if !preset.required.split_whitespace().all(|k| {
        args.get(k)
            .is_some_and(|v| valid_field(k, v) && (v.as_str().is_none_or(|s| !s.trim().is_empty())))
    }) || !args.iter().all(|(k, v)| {
        preset
            .required
            .split_whitespace()
            .chain(preset.optional.split_whitespace())
            .any(|f| f == k)
            && valid_field(k, v)
    }) {
        return None;
    }
    if text(args, "start_mode") == Some("custom_message") && text(args, "frozen_message").is_none()
    {
        return None;
    }
    Some(preset)
}
fn target(invocation: &McpInvocation, preset: &Preset) -> String {
    let mut v = invocation.arguments.as_ref().unwrap_or(&Value::Null);
    for key in preset.target.split('.') {
        v = &v[key];
    }
    clean(v.as_str().unwrap_or(""))
}
/// One display line: terminal controls removed, whitespace collapsed, at most 80 graphemes.
fn clean(text: &str) -> String {
    super::messages::sanitize_user_text(text.into())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .graphemes(true)
        .take(80)
        .collect()
}
pub(super) fn title(invocation: &McpInvocation) -> Option<String> {
    let Some(p) = lookup(invocation) else {
        return tools::title(invocation);
    };
    let target = target(invocation, p);
    if p.tool == "send" {
        let mode = invocation.arguments.as_ref()?["delivery_mode"]
            .as_str()?
            .replace('_', "-");
        return Some(format!("Sending message to {target} · {mode}"));
    }
    Some(if target.is_empty() {
        p.running.into()
    } else {
        format!("{} · {target}", p.running)
    })
}

#[path = "cutex_mcp_receipt.rs"]
mod receipt;
pub(super) use receipt::outcome;
use receipt::Outcome;

#[path = "cutex_mcp_tools.rs"]
mod tools;
pub(super) use tools::previews_detail;

#[cfg(test)]
#[path = "cutex_mcp_display_tests.rs"]
mod tests;
