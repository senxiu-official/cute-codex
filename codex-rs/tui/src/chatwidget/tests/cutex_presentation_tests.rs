use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn cutex_mcp_completed_item_keeps_full_transcript() {
    let (mut chat, mut rx, _ops) = make_chatwidget_manual(/*model_override*/ None).await;
    let _ = drain_insert_history(&mut rx);
    // The same ThreadItem is emitted by direct and CodeMode nested MCP dispatch.
    // Core's code_mode_can_print_structured_mcp_tool_result_fields proves the producer.
    let item = AppServerThreadItem::McpToolCall {
        mcp_app_ui: None,
        id: "exec-nested-call".into(),
        server: "cutex_job".into(),
        tool: "query".into(),
        arguments: json!({"jobId":"job-123"}),
        status: codex_app_server_protocol::McpToolCallStatus::Completed,
        app_context: None,
        mcp_app_resource_uri: None,
        plugin_id: None,
        read_only_hint: None,
        result: Some(Box::new(codex_app_server_protocol::McpToolCallResult {
            content: vec![
                json!({"type":"text","text":json!({"schema":"cutex/job-service-core/v1","jobId":"job-123","revision":1,"state":"running"}).to_string()}),
            ],
            structured_content: None,
            meta: None,
        })),
        error: None,
        duration_ms: Some(1),
    };
    let mut started = item.clone();
    if let AppServerThreadItem::McpToolCall { status, result, .. } = &mut started {
        *status = codex_app_server_protocol::McpToolCallStatus::InProgress;
        *result = None;
    }
    chat.on_mcp_tool_call_started(started);
    assert!(drain_insert_history(&mut rx).is_empty());
    assert!(chat.transcript.active_cell.is_some());
    chat.on_mcp_tool_call_completed(item.clone());
    assert!(chat.transcript.active_cell.is_none());
    let mut observed = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let AppEvent::InsertHistoryCell(cell) = event {
            observed.push((
                lines_to_single_string(&cell.display_lines(80)),
                lines_to_single_string(&cell.transcript_lines(80)),
                lines_to_single_string(&cell.raw_lines()),
            ));
        }
    }
    assert_eq!(observed.len(), 1);
    insta::assert_debug_snapshot!(observed);
    chat.replay_thread_item(item, "turn-1".into(), ReplayKind::ThreadSnapshot);
    let mut replay = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let AppEvent::InsertHistoryCell(cell) = event {
            replay.push((
                lines_to_single_string(&cell.display_lines(80)),
                lines_to_single_string(&cell.transcript_lines(80)),
                lines_to_single_string(&cell.raw_lines()),
            ));
        }
    }
    assert_eq!(replay, observed);
}

#[tokio::test]
async fn cutex_mcp_action_label_live_and_history_reconstruction() {
    fn item(tool: &str, args: serde_json::Value, value: serde_json::Value) -> AppServerThreadItem {
        AppServerThreadItem::McpToolCall {
            mcp_app_ui: None,
            id: tool.into(),
            server: "cutex_job".into(),
            tool: tool.into(),
            arguments: args,
            status: codex_app_server_protocol::McpToolCallStatus::Completed,
            app_context: None,
            mcp_app_resource_uri: None,
            plugin_id: None,
            read_only_hint: None,
            result: Some(Box::new(codex_app_server_protocol::McpToolCallResult {
                content: vec![json!({"type":"text","text":value.to_string()})],
                structured_content: Some(value),
                meta: None,
            })),
            error: None,
            duration_ms: Some(0),
        }
    }
    let job = json!({"schema":"cutex/job-service-core/v1","jobId":"job_123456789012345678901234abcd","revision":1,"state":"running","request":{"actionId":"real-action"}});
    let submit = item(
        "submit",
        json!({"actionId":"real-action","argv":["fixed"],"cwd":"/private"}),
        json!({"status":"committed","deduplicated":false,"job":job}),
    );
    let query = item("query", json!({"jobId":job["jobId"]}), job.clone());
    let output = item(
        "read_output",
        json!({"jobId":job["jobId"],"stream":"stdout"}),
        json!({"jobId":job["jobId"],"stream":"stdout","fromOffset":0,"nextOffset":3,"bytesHex":"616263","gap":false,"truncated":false}),
    );
    for replay in [false, true] {
        let (mut chat, mut rx, _ops) = make_chatwidget_manual(None).await;
        let _ = drain_insert_history(&mut rx);
        for item in [submit.clone(), query.clone(), output.clone()] {
            if replay {
                chat.replay_thread_item(item, "turn".into(), ReplayKind::ThreadSnapshot);
            } else {
                chat.handle_mcp_tool_call_completed_now(item);
            }
        }
        let display = drain_insert_history(&mut rx)
            .into_iter()
            .map(|lines| lines_to_single_string(&lines))
            .collect::<String>();
        assert_eq!(display.matches("real-action").count(), 3);
        assert!(!display.contains("bytesHex"));
        assert!(display.contains("abc"));
    }
}
