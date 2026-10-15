//! Display presets for the cutex agent tools (delegations, notes, alarms, services, agent
//! management) under both their current names (`agent_delegate`, `delegation_wait`, …) and the
//! names older transcripts carry (`agent_ask`, `agent_wait`, `ask_close`, …). Arguments and
//! results are read with the new keys first (`delegation`, `delegation_id`, `delegations`) and the
//! old ones as fallback (`ask`, `ask_id`, `asks`). Presentation only: a result we do not recognise
//! returns `None`, and the cell keeps its upstream rendering.
use super::Outcome;
use super::clean;
use super::super::mcp::McpInvocation;
use serde_json::Map;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Delegate,
    Wait,
    Stop,
    Close,
    Read,
    Progress,
    Answer,
    Tell,
    List,
    AlarmSet,
    AlarmList,
    AlarmCancel,
    Service,
    Management,
}

struct Spec {
    kind: Kind,
    running: &'static str,
    done: &'static str,
    failed: &'static str,
    /// Candidate argument keys for the target, first present wins (dotted paths allowed).
    target: &'static [&'static str],
}

const DELEGATION_KEYS: &[&str] = &["delegation", "delegation_id", "ask", "ask_id"];
const AGENT_KEYS: &[&str] = &[
    "name",
    "agent",
    "target",
    "member",
    "cutex_session_id",
    "spec.name",
    "successor.name",
];

fn spec(tool: &str) -> Option<Spec> {
    let s = |kind, running, done, failed, target| Spec {
        kind,
        running,
        done,
        failed,
        target,
    };
    Some(match tool {
        "agent_delegate" | "agent_ask" => s(
            Kind::Delegate,
            "Delegating to",
            "Delegated to",
            "Delegation not sent to",
            &["target"],
        ),
        "delegation_wait" | "agent_wait" => s(
            Kind::Wait,
            "Waiting for delegations",
            "",
            "Delegation wait failed",
            &[],
        ),
        "delegation_stop" => s(
            Kind::Stop,
            "Stopping delegation",
            "Stopped delegation",
            "Delegation not stopped",
            DELEGATION_KEYS,
        ),
        "delegation_close" | "ask_close" => s(
            Kind::Close,
            "Closing delegation",
            "Closed delegation",
            "Delegation not closed",
            DELEGATION_KEYS,
        ),
        "delegation_read" | "ask_read" => s(
            Kind::Read,
            "Reading delegation result",
            "Read delegation result",
            "Delegation result not read",
            DELEGATION_KEYS,
        ),
        "delegation_progress" | "agent_reply" => s(
            Kind::Progress,
            "Posting delegation progress",
            "Posted delegation progress",
            "Progress not posted",
            DELEGATION_KEYS,
        ),
        "question_answer" | "agent_respond" => s(
            Kind::Answer,
            "Answering question",
            "Answered question",
            "Question not answered",
            &["question", "question_id"],
        ),
        "agent_tell" => s(
            Kind::Tell,
            "Sending note to",
            "Sent note to",
            "Note not sent to",
            &["to"],
        ),
        "agent_list" => s(
            Kind::List,
            "Listing agents",
            "Listed agents",
            "Agent list unavailable",
            &[],
        ),
        "alarm_set" | "set_alarm" => s(
            Kind::AlarmSet,
            "Setting alarm for",
            "Set alarm for",
            "Alarm not set for",
            &["to"],
        ),
        "alarm_list" | "list_alarms" => s(
            Kind::AlarmList,
            "Listing alarms",
            "Listed alarms",
            "Alarms not listed",
            &[],
        ),
        "alarm_cancel" | "cancel_alarm" => s(
            Kind::AlarmCancel,
            "Cancelling alarm",
            "Cancelled alarm",
            "Alarm not cancelled",
            &["alarm_id"],
        ),
        "service_register" | "cutex_service_register" => s(
            Kind::Service,
            "Registering service",
            "Registered service",
            "Service not registered",
            &["slug"],
        ),
        "service_update" | "cutex_service_update" => s(
            Kind::Service,
            "Updating service",
            "Updated service",
            "Service not updated",
            &["slug"],
        ),
        "service_unregister" | "cutex_service_unregister" => s(
            Kind::Service,
            "Unregistering service",
            "Unregistered service",
            "Service not unregistered",
            &["slug"],
        ),
        "service_list" | "cutex_service_list" => s(
            Kind::Service,
            "Listing services",
            "Listed services",
            "Services not listed",
            &[],
        ),
        "agent_create" => s(
            Kind::Management,
            "Creating agent",
            "Created agent",
            "Agent not created",
            AGENT_KEYS,
        ),
        "agent_archive" => s(
            Kind::Management,
            "Archiving agent",
            "Archived agent",
            "Agent not archived",
            AGENT_KEYS,
        ),
        "member_add" => s(
            Kind::Management,
            "Adding member",
            "Added member",
            "Member not added",
            AGENT_KEYS,
        ),
        "member_remove" => s(
            Kind::Management,
            "Removing member",
            "Removed member",
            "Member not removed",
            AGENT_KEYS,
        ),
        "member_runtime" => s(
            Kind::Management,
            "Changing member runtime",
            "Changed member runtime",
            "Member runtime not changed",
            AGENT_KEYS,
        ),
        "director_rotate" => s(
            Kind::Management,
            "Rotating director",
            "Rotated director",
            "Director not rotated",
            &["successor.name", "name", "successor"],
        ),
        _ => return None,
    })
}

fn args(invocation: &McpInvocation) -> Option<&Map<String, Value>> {
    match invocation.arguments.as_ref() {
        None => Some(empty()),
        Some(Value::Null) => Some(empty()),
        Some(value) => value.as_object(),
    }
}

fn empty() -> &'static Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Map::new)
}

fn path<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    let mut parts = key.split('.');
    let mut value = object.get(parts.next()?)?;
    for part in parts {
        value = value.get(part)?;
    }
    value.as_str().filter(|s| !s.trim().is_empty())
}

fn first(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| path(object, key)).map(clean)
}

fn text_of(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| super::super::messages::sanitize_user_text(s.into()).into_owned())
}

fn join(head: &str, target: Option<&str>) -> String {
    match target.filter(|t| !t.is_empty()) {
        Some(target) if head.ends_with(" to") || head.ends_with(" for") => {
            format!("{head} {target}")
        }
        Some(target) => format!("{head} · {target}"),
        None => head.trim_end_matches(" to").trim_end_matches(" for").into(),
    }
}

fn id_list(object: &Map<String, Value>) -> Option<Vec<String>> {
    match object.get("delegations").or_else(|| object.get("asks")) {
        None => Some(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(clean))
            .collect::<Option<Vec<_>>>(),
        Some(_) => None,
    }
}

/// `true` when the summary detail is free text (a brief, a note, a message) that the compact
/// view should preview in a couple of lines.
pub(in crate::history_cell) fn previews_detail(tool: &str) -> bool {
    spec(tool).is_some_and(|spec| {
        matches!(
            spec.kind,
            Kind::Delegate | Kind::Progress | Kind::Tell | Kind::AlarmSet | Kind::Answer
        )
    })
}

pub(super) fn title(invocation: &McpInvocation) -> Option<String> {
    if invocation.server != "cutex" {
        return None;
    }
    let spec = spec(&invocation.tool)?;
    let args = args(invocation)?;
    let target = first(args, spec.target);
    Some(match spec.kind {
        Kind::Delegate => {
            let head = join(spec.running, Some(target.as_deref()?));
            match path(args, "title") {
                Some(title) => format!("{head} · {}", clean(title)),
                None => head,
            }
        }
        Kind::Wait => {
            let ids = id_list(args)?;
            match ids.len() {
                0 => spec.running.into(),
                1..=2 => format!("{} · {}", spec.running, ids.join(", ")),
                n => format!("{} · {n}", spec.running),
            }
        }
        Kind::Close => {
            let (running, _) = close_verbs(args)?;
            join(running, target.as_deref())
        }
        Kind::Read if args.get("artifact").is_some() => {
            join("Reading delegation attachment", target.as_deref())
        }
        Kind::AlarmSet => {
            let head = join(spec.running, Some(target.as_deref()?));
            if let Some(at) = path(args, "at") {
                format!("{head} · {}", clean(at))
            } else if let Some(seconds) = args.get("delay_seconds").and_then(Value::as_u64) {
                format!("{head} · in {seconds}s")
            } else {
                head
            }
        }
        _ => join(spec.running, target.as_deref()),
    })
}

fn close_verbs(args: &Map<String, Value>) -> Option<(&'static str, &'static str)> {
    Some(match args.get("outcome").map(Value::as_str) {
        None => ("Closing delegation", "Closed delegation"),
        Some(Some("stop")) => ("Stopping delegation", "Stopped delegation"),
        Some(Some("accepted")) => ("Accepting delegation", "Accepted delegation"),
        Some(Some("abandoned")) => ("Abandoning delegation", "Abandoned delegation"),
        _ => return None,
    })
}

enum Status {
    Ok,
    Failed(Vec<String>),
    Uncertain,
    OwnerAction,
}

fn status(value: &Value, args: &Map<String, Value>) -> Option<Status> {
    if value["schema"] == "cutex/agent-management/v1" {
        if let Some(id) = args.get("action_id")
            && value["action_id"] != *id
        {
            return None;
        }
        let result = &value["outcome"];
        return Some(match result["status"].as_str()? {
            "complete" => {
                let receipt = &result["receipt"];
                if receipt["schema"] != "cutex/agent-management-receipt/v1"
                    || receipt["action_id"] != value["action_id"]
                {
                    return None;
                }
                Status::Ok
            }
            "no_write" if result["code"] == "response_uncertain" => Status::Uncertain,
            "no_write" => Status::Failed(code_detail(result)),
            "owner_action_required" => Status::OwnerAction,
            _ => return None,
        });
    }
    match value["ok"].as_bool()? {
        true => Some(Status::Ok),
        false
            if value["status"] == "response_uncertain"
                || value["code"] == "outcome_unknown" =>
        {
            Some(Status::Uncertain)
        }
        false => {
            let detail = code_detail(value);
            (!detail.is_empty()).then_some(Status::Failed(detail))
        }
    }
}

fn code_detail(value: &Value) -> Vec<String> {
    let Some(code) = value["code"]
        .as_str()
        .filter(|code| !code.is_empty() && !code.chars().any(char::is_control))
    else {
        return Vec::new();
    };
    let mut detail = vec![format!("Code: {code}")];
    if let Some(text) = text_of(&value["detail"]).or_else(|| text_of(&value["message"])) {
        detail.push(text);
    }
    detail
}

fn outcome_of(text: String, detail: Vec<String>) -> Outcome {
    Outcome {
        text,
        job_id: None,
        occurred_at_millis: None,
        detail,
        output: None,
        failed: false,
        uncertain: false,
    }
}

fn count(value: &Value, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(Value::as_array)
        .map(Vec::len)
}

pub(super) fn outcome(invocation: &McpInvocation, body: &str) -> Option<Outcome> {
    if invocation.server != "cutex" {
        return None;
    }
    let spec = spec(&invocation.tool)?;
    title(invocation)?;
    let args = args(invocation)?;
    let value: Value = serde_json::from_str(body).ok()?;
    let target = first(args, spec.target).or_else(|| {
        matches!(
            spec.kind,
            Kind::Stop | Kind::Close | Kind::Read | Kind::Progress
        )
        .then(|| first(value.as_object()?, &["delegation_id", "ask_id"]))
        .flatten()
    });
    let target = target.as_deref();
    match status(&value, args)? {
        Status::Uncertain => {
            let mut outcome = outcome_of(join("Outcome uncertain", target), Vec::new());
            outcome.uncertain = true;
            return Some(outcome);
        }
        Status::OwnerAction => {
            let mut outcome = outcome_of(join("Owner action required", target), Vec::new());
            outcome.failed = true;
            return Some(outcome);
        }
        Status::Failed(detail) => {
            let mut outcome = outcome_of(join(spec.failed, target), detail);
            outcome.failed = true;
            return Some(outcome);
        }
        Status::Ok => {}
    }
    let mut detail = Vec::new();
    let text = match spec.kind {
        Kind::Delegate => {
            let name = text_of(&value["target"]).map(|name| clean(&name));
            let mut head = join(spec.done, name.as_deref().or(target));
            if let Some(title) = path(args, "title").or_else(|| value["title"].as_str()) {
                head = format!("{head} · {}", clean(title));
            }
            if let Some(brief) = args.get("text").and_then(text_of) {
                detail.push(brief);
            }
            if let Some(state) = value["state"].as_str() {
                detail.push(format!("State: {}", clean(state)));
            }
            if let Some(id) = first(value.as_object()?, &["delegation_id", "ask_id"]) {
                detail.push(format!("Delegation: {id}"));
            }
            head
        }
        Kind::Wait => {
            let results = value["results"].as_array()?.len();
            let reason = value["reason"].as_str().map(clean);
            for item in ["delegations", "asks"]
                .iter()
                .find_map(|key| value[*key].as_array())
                .into_iter()
                .flatten()
            {
                let who = text_of(&item["target"]).map(|name| clean(&name));
                let state = item["state"].as_str().map(clean);
                if let (Some(who), Some(state)) = (who, state) {
                    match item["title"].as_str().filter(|t| !t.trim().is_empty()) {
                        Some(title) => detail.push(format!("{who} · {}: {state}", clean(title))),
                        None => detail.push(format!("{who}: {state}")),
                    }
                }
            }
            match (results, reason.as_deref()) {
                (1, _) => "Received 1 delegation result".into(),
                (n, _) if n > 1 => format!("Received {n} delegation results"),
                (_, Some("timeout")) => "No delegation result yet".into(),
                (_, Some(reason)) => format!("Delegation wait ended · {reason}"),
                (_, None) => "Delegation wait ended".into(),
            }
        }
        Kind::Stop => {
            if value["stopping"] == true {
                join("Stop requested", target)
            } else {
                join(spec.done, target)
            }
        }
        Kind::Close => {
            let (_, done) = close_verbs(args)?;
            if value["stopping"] == true {
                join("Stop requested", target)
            } else {
                join(done, target)
            }
        }
        Kind::Read => {
            if let Some(status) = value["status"].as_str() {
                detail.push(format!("Status: {}", clean(status)));
            }
            if args.get("artifact").is_some() {
                join("Read delegation attachment", target)
            } else {
                join(spec.done, target)
            }
        }
        Kind::Progress => {
            if let Some(text) = args.get("text").and_then(text_of) {
                detail.push(text);
            }
            join(spec.done, target)
        }
        Kind::Answer => {
            if let Some(answers) = args.get("answers").and_then(Value::as_object) {
                let joined = answers
                    .values()
                    .filter_map(|entry| entry["answers"].as_array())
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; ");
                if let Some(joined) = text_of(&Value::String(joined)) {
                    detail.push(joined);
                }
            }
            join(spec.done, target)
        }
        Kind::Tell => {
            let name = text_of(&value["name"]).map(|name| clean(&name));
            if let Some(text) = args.get("text").and_then(text_of) {
                detail.push(text);
            }
            join(spec.done, name.as_deref().or(target))
        }
        Kind::List => {
            let agents = value["agents"].as_array()?.len();
            for (keys, label) in [
                (&["my_delegations", "my_asks"][..], "Your open delegations"),
                (&["delegations_to_me", "asks_to_me"][..], "Delegations to you"),
                (&["open_questions"][..], "Open questions"),
            ] {
                if let Some(n) = count(&value, keys).filter(|n| *n > 0) {
                    detail.push(format!("{label}: {n}"));
                }
            }
            if let Some(n) = value["unclaimed_results"].as_u64().filter(|n| *n > 0) {
                detail.push(format!("Unclaimed results: {n}"));
            }
            format!("{} · {agents} visible", spec.done)
        }
        Kind::AlarmSet => {
            if let Some(message) = args.get("message").and_then(text_of) {
                detail.push(message);
            }
            let head = join(spec.done, target);
            match value["result"]["at"].as_str().filter(|at| !at.trim().is_empty()) {
                Some(at) => format!("{head} · {}", clean(at)),
                None => head,
            }
        }
        Kind::AlarmList => match value["result"].as_array() {
            Some(rows) => format!("{} · {}", spec.done, rows.len()),
            None => spec.done.into(),
        },
        Kind::AlarmCancel | Kind::Service | Kind::Management => join(spec.done, target),
    };
    Some(outcome_of(text, detail))
}
