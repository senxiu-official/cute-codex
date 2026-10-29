use super::*;
use crate::history_cell::HistoryCell;
use crate::history_cell::mcp::McpToolCallCell;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::time::Duration;

fn cell(server: &str, tool: &str, args: Value) -> McpToolCallCell {
    McpToolCallCell::new(
        "nested-exec-id".into(),
        McpInvocation {
            server: server.into(),
            tool: tool.into(),
            arguments: Some(args),
        },
        false,
    )
}
fn finish(call: &mut McpToolCallCell, value: Value, failed: bool) {
    call.complete(
        Duration::ZERO,
        Ok(codex_protocol::mcp::CallToolResult {
            content: vec![json!({"type":"text","text":value.to_string()})],
            structured_content: Some(value),
            is_error: Some(failed),
            meta: None,
        }),
    );
}
fn render(lines: Vec<ratatui::text::Line<'static>>) -> String {
    lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}
// Native 0.156 previews compact JSON and cap output, while transcripts retain raw text.
// A rejected Cutex receipt must therefore use the native invocation header, not a state label.
fn assert_upstream_fallback(call: &McpToolCallCell) {
    let display = render(call.display_lines(80));
    assert!(
        display.starts_with("• Called") || display.starts_with("• Calling"),
        "{display}"
    );
    assert!(!display.contains("State:"), "{display}");
    let transcript = render(call.transcript_lines(80));
    assert!(transcript.starts_with("• Called") || transcript.starts_with("• Calling"));
}
fn job(state: &str) -> Value {
    json!({"schema":"cutex/job-service-core/v1","jobId":"job-1","revision":1,"state":state,"request":{"actionId":"action-🦀"}})
}
#[test]
fn job_lifecycle_pink_and_full_invocation() {
    let mut call = cell(
        "cutex_job",
        "submit",
        json!({"actionId":"action-🦀","argv":["echo","full argument retained"],"cwd":"/private"}),
    );
    let running = render(call.display_lines(40));
    finish(
        &mut call,
        json!({"status":"committed","job":job("launch_pending"),"deduplicated":false}),
        false,
    );
    let detailed = (call.transcript_lines(120), call.raw_lines());
    let display = call.display_lines(40);
    assert!(
        display
            .iter()
            .find(|line| line.to_string().contains("State:"))
            .unwrap()
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::DIM)
    );
    assert_eq!(
        display[0].spans[0].style.fg,
        Some(crate::terminal_palette::rgb_color((0xE0, 0x8E, 0xB2)))
    );
    assert!(render(detailed.1.clone()).contains("full argument retained"));
    let narrow = render(call.display_lines(20));
    assert_eq!((call.transcript_lines(120), call.raw_lines()), detailed);
    insta::assert_snapshot!(format!(
        "running:\n{running}\nnormal:\n{}\nnarrow:\n{narrow}\nraw:\n{}",
        render(display),
        render(detailed.1)
    ));
}
#[test]
fn known_states_never_upgrade_transport_success() {
    let mut call = cell("cutex_job", "query", json!({"jobId":"job-1"}));
    finish(&mut call, json!({"unknown":"success"}), false);
    assert_upstream_fallback(&call);
    finish(&mut call, job("launch_unknown"), false);
    assert!(render(call.display_lines(80)).contains("launch_unknown"));
    assert_ne!(
        call.display_lines(80)[0].spans[0].style.fg,
        Some(crate::terminal_palette::rgb_color((0xE0, 0x8E, 0xB2)))
    );
    finish(&mut call, job("failed"), false);
    assert_eq!(
        call.display_lines(80)[0].spans[0].style.fg,
        Some(ratatui::style::Color::Red)
    );
    let mut future = job("running");
    future["schema"] = json!("cutex/job-service-core/v2");
    finish(&mut call, future, false);
    assert_upstream_fallback(&call);
    let mut wrong = job("running");
    wrong["jobId"] = json!("other");
    finish(&mut call, wrong, false);
    assert_upstream_fallback(&call);
}

#[test]
fn task_service_contract_read_uses_compact_task_presentation() {
    let args = json!({"assignment_id":"assignment-1"});
    let call = cell("cutex", "cutex_task_service_read", args.clone());
    let running = render(call.display_lines(80));
    assert!(
        running.starts_with("• Reading task contract · assignment-1"),
        "{running}"
    );

    let mut completed = cell("cutex", "cutex_task_service_read", args);
    finish(
        &mut completed,
        json!({
            "schema":"cutex/task-service-query-response/v2",
            "outcome":{
                "kind":"contract",
                "body":{
                    "task_id":"task-1",
                    "task_revision":1,
                    "contract_sha256":"a".repeat(64)
                }
            }
        }),
        false,
    );
    let rendered = render(completed.display_lines(80));
    assert!(
        rendered.starts_with("• Read task contract · assignment-1"),
        "{rendered}"
    );
    assert!(rendered.contains("Task: task-1"), "{rendered}");
    assert!(rendered.contains("Revision: 1"), "{rendered}");
    assert!(rendered.contains("Contract SHA-256:"), "{rendered}");

    let mut unavailable = cell(
        "cutex",
        "cutex_task_service_read",
        json!({"assignment_id":"assignment-1"}),
    );
    finish(
        &mut unavailable,
        json!({
            "schema":"cutex/task-service-query-response/v2",
            "outcome":{"kind":"no_write","body":{"code":"not_found","detail":"absent"}}
        }),
        true,
    );
    let rendered = render(unavailable.display_lines(80));
    assert!(
        rendered.starts_with("• Task contract not read · assignment-1"),
        "{rendered}"
    );
    assert!(rendered.contains("Code: not_found"), "{rendered}");

    let mut unknown = cell(
        "cutex",
        "cutex_task_service_read",
        json!({"assignment_id":"assignment-1"}),
    );
    finish(
        &mut unknown,
        json!({"schema":"cutex/task-service-query-response/v2","outcome":{"kind":"snapshot"}}),
        false,
    );
    assert_upstream_fallback(&unknown);
}

#[test]
fn registry_actions_and_shape_fallback() {
    let spec = json!({"name":"worker","cwd":"/private","profile":"p","runtime_backend":"native","model":"test","reasoning":"low","permissions":"read-only","approval_policy":"on-request","sandbox_mode":"read-only","groups":[]});
    let mut headers = Vec::new();
    for op in [
        "create",
        "query_managed",
        "online",
        "offline",
        "restart",
        "close",
        "replace",
        "director_rotate",
    ] {
        let mut args = json!({"operation":op,"action_id":"a"});
        match op {
            "create" => {
                args["spec"] = spec.clone();
                args["start_mode"] = json!("bootstrap_only");
            }
            "query_managed" => {}
            "replace" => {
                args["predecessor_cutex_session_id"] = json!("old");
                args["policy"] = json!("close_before_create");
                args["successor"] = spec.clone();
                args["start_mode"] = json!("bootstrap_only");
            }
            "director_rotate" => {
                args["expected_predecessor_cutex_session"] = json!("old");
                args["expected_authority_epoch"] = json!(1);
                args["mode"] = json!("retain_predecessor_bootstrap_only");
                args["successor"] = spec.clone();
            }
            _ => args["cutex_session_id"] = json!("worker"),
        }
        headers.push(render(
            cell("cutex", "cutex_agent_management", args.clone()).display_lines(80),
        ));
        let mut completed = cell("cutex", "cutex_agent_management", args.clone());
        let kind = match op {
            "create" => "created",
            "query_managed" => "query_managed",
            "replace" => "replaced",
            "director_rotate" => "director_rotated",
            _ => "lifecycle",
        };
        finish(
            &mut completed,
            json!({"schema":"cutex/agent-management/v1","action_id":"a","outcome":{"status":"complete","receipt":{"schema":"cutex/agent-management-receipt/v1","action_id":"a","operation":op,"result":{"kind":kind}}}}),
            false,
        );
        assert_ne!(completed.display_lines(80), completed.transcript_lines(80));
        headers.push(
            render(completed.display_lines(80))
                .lines()
                .next()
                .unwrap()
                .into(),
        );
        args["unreviewed_field"] = json!(true);
        let fallback = cell("cutex", "cutex_agent_management", args);
        assert_eq!(fallback.display_lines(80), fallback.transcript_lines(80));
    }
    assert!(headers.contains(&"• Rotating director · worker".to_owned()));
    assert!(headers.contains(&"• Rotated director · worker".to_owned()));
    assert!(!headers.contains(&"• Rotated director · old".to_owned()));
    for op in [
        "start",
        "report_status",
        "block",
        "resume",
        "submit",
        "decline",
        "abort_attempt",
    ] {
        let mut args = json!({"operation":op,"action_id":"a","assignment_id":"assignment"});
        if matches!(op, "report_status" | "block") {
            args["summary"] = json!("status");
        }
        if op == "submit" {
            args["result_sha256"] = json!("a".repeat(64));
            args["result_reference"] = json!("result");
        }
        let mut call = cell("cutex", "cutex_task_service", args);
        headers.push(render(call.display_lines(80)));
        finish(
            &mut call,
            json!({"schema":"cutex/task-service-tool-receipt/v1","status":"committed"}),
            false,
        );
        headers.push(
            render(call.display_lines(80))
                .lines()
                .next()
                .unwrap()
                .to_owned(),
        );
    }
    insta::assert_snapshot!(headers.join("\n"));
    for (server, tool, args) in [
        ("other", "query", json!({"jobId":"j"})),
        ("cutex_job_extra", "query", json!({"jobId":"j"})),
        ("cutex_job", "query", json!({"jobId":3})),
        (
            "cutex_job",
            "read_output",
            json!({"jobId":"j","stream":"all"}),
        ),
        ("cutex", "cutex_agent_list", json!({"all_hosts":true})),
        (
            "cutex",
            "cutex_task_service",
            json!({"operation":"invented","action_id":"a","assignment_id":"x"}),
        ),
    ] {
        let call = cell(server, tool, args);
        assert_upstream_fallback(&call);
    }
}
#[test]
fn message_receipt_resolves_durable_target_to_name_and_bounds_preview() {
    let args = json!({"to":"cutex.worker-id","message":"long 私有 preview ".repeat(100),"external_message_id":"m","delivery_mode":"after_turn"});
    let mut call = cell("cutex", "send", args);
    finish(
        &mut call,
        json!({"ok":true,"to":"stock.runtime-id","to_cutex_session_id":"cutex.worker-id","to_name":"worker-name","id":"m","queued":true}),
        false,
    );
    let display = render(call.display_lines(80));
    assert!(display.contains("Sent message to worker-name · after-turn"));
    assert!(!display.contains("stock.runtime-id"));
    assert!(display.lines().count() <= 4, "{display}");
    assert!(render(call.transcript_lines(80)).contains("long 私有 preview"));
}

#[test]
fn messages_receipts_and_terminal_controls() {
    let args = json!({"to":"worker\u{1b}[31m\n二","message":"body","external_message_id":"m","delivery_mode":"soon"});
    let mut call = cell("cutex", "send", args.clone());
    finish(
        &mut call,
        json!({"ok":true,"to":args["to"],"id":"m","queued":true}),
        false,
    );
    let display = render(call.display_lines(30));
    assert!(!display.contains('\u{1b}'));
    assert!(display.contains("Sent message to"));
    insta::assert_snapshot!(display);
    let mut management = cell(
        "cutex",
        "cutex_agent_management",
        json!({"operation":"restart","action_id":"a","cutex_session_id":"w"}),
    );
    finish(
        &mut management,
        json!({"schema":"cutex/agent-management/v1","action_id":"a","outcome":{"status":"no_write","code":"response_uncertain"}}),
        true,
    );
    assert!(render(management.display_lines(80)).starts_with("• Outcome uncertain"));
    insta::assert_snapshot!(
        "no_bullet",
        render(crate::history_cell::event_presentation::render_event(
            Some("Notice · workspace"),
            &["Plain body\u{1b}[31m\nsecond line".into()],
            None,
            24
        ))
    );
}

#[test]
fn director_terminal_and_observation_receipts() {
    let mut headers = Vec::new();
    for tool in ["cutex_task_service_director", "cutex_task_service_terminal"] {
        let operations: &[&str] = if tool.ends_with("terminal") {
            &["accept_result", "request_changes", "fail_result"]
        } else {
            &[
                "create_revision",
                "assign",
                "create_and_assign",
                "query",
                "accept_result",
                "request_changes",
                "fail_result",
                "cancel",
            ]
        };
        for op in operations {
            let mut args = json!({"operation":op,"action_id":"action"});
            match *op {
                "create_revision" | "create_and_assign" => {
                    args["project_id"] = json!("p");
                    args["workflow_id"] = json!("w");
                    args["task_id"] = json!("t");
                    args["task_revision"] = json!(1);
                    args["opaque_contract"] = json!("full contract");
                    args["completion_policy"] = json!("director_acceptance");
                }
                "query" => args["selector"] = json!({"kind":"all"}),
                _ => {}
            }
            if matches!(*op, "assign" | "create_and_assign") {
                args["project_id"] = json!("p");
                args["task_id"] = json!("t");
                args["task_revision"] = json!(1);
                args["assignment_id"] = json!("assignment");
                args["assignee_cutex_session_id"] = json!("worker");
                args["summary"] = json!("full summary");
            }
            if matches!(
                *op,
                "accept_result" | "request_changes" | "fail_result" | "cancel"
            ) {
                args["assignment_id"] = json!("assignment");
            }
            if *op == "request_changes" {
                args["decision_reference"] = json!("review");
            }
            let mut call = cell("cutex", tool, args);
            assert_ne!(call.display_lines(80), call.transcript_lines(80));
            headers.push(render(call.display_lines(80)));
            let receipt = json!({"schema":if tool.ends_with("terminal") { "cutex/task-service-terminal-tool-receipt/v1" } else { "cutex/task-service-director-tool-receipt/v1" },"status":"committed","action_id":"action"});
            finish(&mut call, receipt.clone(), false);
            headers.push(
                render(call.display_lines(80))
                    .lines()
                    .next()
                    .unwrap()
                    .into(),
            );
            let mut wrong = receipt;
            wrong["action_id"] = json!("other");
            finish(&mut call, wrong, false);
            assert_upstream_fallback(&call);
        }
    }
    for (tool, args, result) in [
        (
            "cutex_agent_list",
            json!({}),
            json!({"ok":true,"scope":"local_group_visible","agents":[]}),
        ),
        (
            "query_managed",
            json!({"action_id":"a"}),
            json!({"schema":"cutex/agent-management/v1","action_id":"a","outcome":{"status":"complete","receipt":{"schema":"cutex/agent-management-receipt/v1","action_id":"a","operation":"query_managed","result":{"kind":"query_managed"}}}}),
        ),
    ] {
        let mut call = cell("cutex", tool, args);
        finish(&mut call, result, false);
        assert_ne!(call.display_lines(80), call.transcript_lines(80));
        headers.push(
            render(call.display_lines(80))
                .lines()
                .next()
                .unwrap()
                .into(),
        );
    }
    for (tool, args, result) in [
        (
            "cancel",
            json!({"jobId":"job-1","expectedRevision":1}),
            job("cancelled"),
        ),
        (
            "read_output",
            json!({"jobId":"job-1","stream":"stdout"}),
            json!({"jobId":"job-1","stream":"stdout","fromOffset":0,"nextOffset":1,"bytesHex":"61","gap":false,"truncated":false}),
        ),
    ] {
        let mut call = cell("cutex_job", tool, args);
        finish(&mut call, result, false);
        assert_ne!(call.display_lines(80), call.transcript_lines(80));
        headers.push(
            render(call.display_lines(80))
                .lines()
                .next()
                .unwrap()
                .into(),
        );
    }
    insta::assert_snapshot!(headers.join("\n"));
}

#[test]
fn transport_error_and_contradictory_receipt() {
    let mut call = cell("cutex_job", "query", json!({"jobId":"job-1"}));
    call.complete(Duration::ZERO, Err("connection lost".into()));
    insta::assert_snapshot!(render(call.display_lines(40)));
    finish(&mut call, job("running"), true);
    assert_upstream_fallback(&call);
}

#[test]
fn conflicting_structured_result_preserves_upstream_detail() {
    let mut call = cell("cutex_job", "query", json!({"jobId":"job-1"}));
    call.complete(
        Duration::ZERO,
        Ok(codex_protocol::mcp::CallToolResult {
            content: vec![json!({"type":"text","text":job("running").to_string()})],
            structured_content: Some(job("failed")),
            is_error: Some(false),
            meta: None,
        }),
    );
    assert_upstream_fallback(&call);
}

#[test]
fn submit_action_survives_ambiguous_history_label() {
    let mut call = cell(
        "cutex_job",
        "submit",
        json!({"actionId":"action-🦀","argv":["fixed"],"cwd":"/private"}),
    );
    call.set_job_label("job-1".into());
    finish(
        &mut call,
        json!({"status":"committed","deduplicated":false,"job":job("running")}),
        false,
    );
    assert!(render(call.display_lines(80)).contains("Submitted job · action-🦀 · job-1"));
}

#[test]
fn send_http_failure_uses_message_display_and_preserves_raw_receipt() {
    let mut call = cell(
        "cutex",
        "send",
        json!({
            "to":"cutex.worker", "message":"Check the checkpoint", "external_message_id":"probe",
            "delivery_mode":"soon"
        }),
    );
    let receipt = json!({"http_status":409,"error":{
        "code":"target_unavailable","message":"Target has no current online endpoint","ok":false
    }});
    call.complete(
        Duration::ZERO,
        Ok(codex_protocol::mcp::CallToolResult {
            content: vec![json!({"type":"text","text":receipt.to_string()})],
            structured_content: None,
            is_error: Some(true),
            meta: None,
        }),
    );
    let display = render(call.display_lines(100));
    assert!(display.contains("Message request failed to cutex.worker · soon"));
    assert!(display.contains("Target has no current online endpoint"));
    assert!(!display.contains("Called"));
    assert!(render(call.raw_lines()).contains("target_unavailable"));
}

#[test]
fn delegation_tools_new_and_old_names() {
    let view = |id: &str| json!({"ok":true,"created":true,"delegation_id":id,"target":"Kimi","title":"Parser review","state":"queued","detail":"Queued"});
    let old_view = |id: &str| json!({"ok":true,"created":true,"ask_id":id,"target":"Kimi","state":"pendingReview"});
    let cases: Vec<(&str, Value, Value)> = vec![
        (
            "agent_delegate",
            json!({"target":"Kimi","title":"Parser review","text":"Review the parser\nand report","notify":"turn"}),
            view("del-1"),
        ),
        (
            "agent_ask",
            json!({"target":"cutex.kimi","text":"Review the parser","review":true}),
            old_view("ask-1"),
        ),
        (
            "delegation_wait",
            json!({"delegations":["del-1"],"timeout_ms":1000}),
            json!({"ok":true,"reason":"result","results":[{}],"delegations":[{"target":"Kimi","title":"Parser review","state":"completed"}]}),
        ),
        (
            "agent_wait",
            json!({"asks":["ask-1","ask-2","ask-3"]}),
            json!({"ok":true,"reason":"timeout","results":[],"asks":[{"target":"Kimi","state":"running"}]}),
        ),
        (
            "delegation_stop",
            json!({"delegation":"del-1"}),
            json!({"ok":true,"delegation_id":"del-1","stopping":true}),
        ),
        (
            "delegation_close",
            json!({"delegation":"del-1","outcome":"accepted"}),
            json!({"ok":true,"delegation_id":"del-1","stopping":false}),
        ),
        (
            "ask_close",
            json!({"ask":"ask-1","outcome":"stop"}),
            json!({"ok":true,"ask_id":"ask-1","stopping":false}),
        ),
        (
            "ask_close",
            json!({"ask":"ask-1","outcome":"abandoned"}),
            json!({"ok":false,"status":"no_write","code":"ask_closed","detail":"the ask is already closed"}),
        ),
        (
            "delegation_read",
            json!({"delegation":"del-1","cursor":8192}),
            json!({"ok":true,"delegation_id":"del-1","status":"completed","text":"page"}),
        ),
        (
            "ask_read",
            json!({"ask":"ask-1","artifact":"art-1"}),
            json!({"ok":true,"ask_id":"ask-1","artifact":"art-1","text":"x"}),
        ),
        (
            "delegation_progress",
            json!({"text":"halfway"}),
            json!({"ok":true,"delegation_id":"del-1"}),
        ),
        (
            "agent_reply",
            json!({"ask":"ask-1","text":"halfway"}),
            json!({"ok":true,"ask_id":"ask-1"}),
        ),
        (
            "question_answer",
            json!({"question":"q-1","answers":{"q0":{"answers":["sqlite"]}}}),
            json!({"ok":true,"question_id":"q-1","delegation_id":"del-1"}),
        ),
        (
            "agent_respond",
            json!({"question":"q-1","answers":{"q0":{"answers":["sqlite","wal"]}}}),
            json!({"ok":true,"question_id":"q-1","ask_id":"ask-1"}),
        ),
        (
            "agent_tell",
            json!({"to":"cutex.kimi","text":"fyi: main is green"}),
            json!({"ok":true,"to":"cutex.kimi","name":"Kimi"}),
        ),
        (
            "agent_tell",
            json!({"to":"Kimi","text":"x"}),
            json!({"ok":false,"status":"response_uncertain","code":"outcome_unknown"}),
        ),
        (
            "agent_list",
            json!({}),
            json!({"ok":true,"agents":[{},{}],"my_delegations":[{}],"delegations_to_me":[],"open_questions":[],"unclaimed_results":1}),
        ),
        (
            "agent_list",
            json!({"include_offline":true}),
            json!({"ok":true,"agents":[{}],"my_asks":[{},{}],"asks_to_me":[{}]}),
        ),
        (
            "alarm_set",
            json!({"alarm_id":"a1","to":"Kimi","message":"check CI","delay_seconds":600}),
            json!({"ok":true,"result":{"alarm_id":"a1","at":"2026-10-08T12:00:00Z"}}),
        ),
        (
            "set_alarm",
            json!({"alarm_id":"a1","to":"Kimi","message":"check CI","at":"2026-10-08 12:00"}),
            json!({"ok":false,"code":"alarm_rejected","detail":"Target not visible to caller"}),
        ),
        ("alarm_list", json!({}), json!({"ok":true,"result":[{},{}]})),
        (
            "cancel_alarm",
            json!({"alarm_id":"a1"}),
            json!({"ok":true,"result":{}}),
        ),
        (
            "service_register",
            json!({"slug":"myapp-web","runtime":"external"}),
            json!({"ok":true}),
        ),
        (
            "cutex_service_list",
            json!({"mine":true}),
            json!({"ok":true}),
        ),
        (
            "agent_create",
            json!({"name":"worker","profile":"p"}),
            json!({"ok":true}),
        ),
        (
            "agent_archive",
            json!({"agent":"worker","action_id":"a"}),
            json!({"schema":"cutex/agent-management/v1","action_id":"a","outcome":{"status":"complete","receipt":{"schema":"cutex/agent-management-receipt/v1","action_id":"a"}}}),
        ),
        ("member_add", json!({"member":"worker"}), json!({"ok":true})),
        (
            "member_remove",
            json!({"member":"worker"}),
            json!({"ok":false,"code":"not_a_member","detail":"worker is not a member"}),
        ),
        (
            "member_runtime",
            json!({"member":"worker","action":"restart"}),
            json!({"ok":true}),
        ),
        (
            "director_rotate",
            json!({"successor":{"name":"next"}}),
            json!({"ok":true}),
        ),
    ];
    let mut out = Vec::new();
    for (tool, args, result) in cases {
        let mut call = cell("cutex", tool, args);
        let running = render(call.display_lines(100));
        assert_eq!(running.lines().count(), 1, "{tool}: {running}");
        out.push(format!("[{tool}]"));
        out.push(running);
        let failed = result["ok"] == false;
        finish(&mut call, result, failed);
        let done = render(call.display_lines(100));
        assert!(!done.contains("Called"), "{tool}: {done}");
        out.push(done);
    }
    insta::assert_snapshot!(out.join("\n"));
}

#[test]
fn delegation_tools_fall_back_on_unknown_shapes() {
    // Unknown result shapes keep the upstream rendering.
    for (tool, args, result) in [
        (
            "agent_delegate",
            json!({"target":"Kimi","text":"x"}),
            json!({"weird":true}),
        ),
        ("delegation_wait", json!({}), json!({"ok":true})),
        ("agent_list", json!({}), json!({"ok":true})),
        (
            "delegation_close",
            json!({"delegation":"d","outcome":"invented"}),
            json!({"ok":true}),
        ),
        (
            "agent_tell",
            json!({"to":"Kimi","text":"x"}),
            json!({"ok":false}),
        ),
        (
            "agent_archive",
            json!({"agent":"w","action_id":"a"}),
            json!({"schema":"cutex/agent-management/v1","action_id":"other","outcome":{"status":"complete"}}),
        ),
    ] {
        let mut call = cell("cutex", tool, args);
        finish(&mut call, result, false);
        assert_upstream_fallback(&call);
    }
    // Other servers, missing targets and malformed arguments are not ours.
    for (server, tool, args) in [
        (
            "other",
            "agent_delegate",
            json!({"target":"Kimi","text":"x"}),
        ),
        ("cutex", "agent_delegate", json!({"text":"x"})),
        ("cutex", "agent_wait", json!({"asks":"ask-1"})),
        ("cutex", "agent_list", json!(["x"])),
        ("cutex", "alarm_set", json!({"alarm_id":"a","message":"m"})),
    ] {
        let call = cell(server, tool, args);
        assert_upstream_fallback(&call);
    }
    // A long brief is previewed; the full text stays in the transcript.
    let mut call = cell(
        "cutex",
        "agent_delegate",
        json!({"target":"Kimi\u{1b}[31m","text":"long 私有 brief ".repeat(100)}),
    );
    finish(
        &mut call,
        json!({"ok":true,"delegation_id":"del-1","target":"Kimi","state":"queued"}),
        false,
    );
    let display = render(call.display_lines(80));
    assert!(display.starts_with("• Delegated to Kimi"), "{display}");
    assert!(!display.contains('\u{1b}'));
    assert!(display.lines().count() <= 6, "{display}");
    assert!(render(call.transcript_lines(80)).contains("long 私有 brief"));
}
