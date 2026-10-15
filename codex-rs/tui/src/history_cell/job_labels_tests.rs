use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
fn receipt(id: &str, action: &str) -> ThreadItem {
    ThreadItem::McpToolCall {
        mcp_app_ui: None,
        id: format!("call-{id}"),
        server: "cutex_job".into(),
        tool: "submit".into(),
        arguments: json!({"actionId":action,"argv":["fixed"],"cwd":"/private"}),
        status: codex_app_server_protocol::McpToolCallStatus::Completed,
        app_context: None,
        mcp_app_resource_uri: None,
        plugin_id: None,
        read_only_hint: None,
        result: Some(Box::new(codex_app_server_protocol::McpToolCallResult {
            content: vec![
                json!({"type":"text","text":json!({"status":"committed","deduplicated":false,"job":{"schema":"cutex/job-service-core/v1","jobId":id,"revision":1,"state":"running","request":{"actionId":action}}}).to_string()}),
            ],
            structured_content: None,
            meta: None,
        })),
        error: None,
        duration_ms: Some(0),
    }
}
#[test]
fn job_labels_replay_conflict_and_short_id_collision() {
    let one = "job_1234567890000000000000000abcd";
    let two = "job_1234567891111111111111111abcd";
    let mut live = JobLabels::default();
    let mut replay = JobLabels::default();
    for item in [
        receipt(one, "actual-action"),
        receipt(one, "actual-action"),
        receipt(two, "another-action"),
    ] {
        let _ = live.observe(&item);
        let _ = replay.observe(&item);
    }
    assert_eq!(live.label(one), replay.label(one));
    assert_eq!(live.label(one), format!("actual-action · {one}"));
    assert_eq!(live.label(two), format!("another-action · {two}"));
    let _ = live.observe(&receipt(one, "conflicting-action"));
    assert_eq!(live.label(one), one);
    assert_eq!(JobLabels::default().label(one), "job_12345678…abcd");
}
#[test]
fn job_labels_unknown_result_or_server_never_creates_map() {
    let mut labels = JobLabels::default();
    let mut item = receipt("job-1", "action");
    if let ThreadItem::McpToolCall { server, .. } = &mut item {
        *server = "cutex_job_impostor".into();
    }
    let _ = labels.observe(&item);
    assert_eq!(labels.label("job-1"), "job-1");
    let mut item = receipt("job-1", "action");
    if let ThreadItem::McpToolCall { arguments, .. } = &mut item {
        arguments["actionId"] = json!("different");
    }
    let _ = labels.observe(&item);
    assert_eq!(labels.label("job-1"), "job-1");
}
