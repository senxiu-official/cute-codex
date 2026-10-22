use super::*;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputPayload;
use pretty_assertions::assert_eq;

const NO_GOAL: &str = "cannot update goal because this thread has no goal";

fn function_key(name: &str, arguments: &str) -> ToolCallKey {
    ToolCallKey::new(
        &ToolName::plain(name),
        &ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    )
}

fn fail_times(failures: &RepeatedToolFailures, key: &ToolCallKey, error: &str, times: u32) {
    for _ in 0..times {
        assert_eq!(None, failures.refusal(key));
        failures.record(key.clone(), Some(error.to_string()));
    }
}

#[test]
fn identical_failures_are_refused_then_stop_the_turn() {
    let failures = RepeatedToolFailures::default();
    let key = function_key("update_goal", r#"{"status":"complete"}"#);
    fail_times(&failures, &key, NO_GOAL, MAX_IDENTICAL_TOOL_FAILURES);

    for _ in 0..MAX_REFUSED_REPEATS {
        let Some(message) = failures.refusal(&key) else {
            panic!("expected refusal");
        };
        assert!(message.contains("`update_goal`"), "{message}");
        assert!(message.contains("failed 3 times"), "{message}");
        assert!(message.contains(NO_GOAL), "{message}");
        assert_eq!(None, failures.take_stop_request());
    }
    assert!(failures.refusal(&key).is_some());
    let Some(message) = failures.take_stop_request() else {
        panic!("expected a turn stop request");
    };
    assert!(message.contains("`update_goal`"), "{message}");
    // The stop request is delivered once.
    assert_eq!(None, failures.take_stop_request());
}

#[test]
fn argument_key_order_and_whitespace_do_not_matter() {
    assert_eq!(
        function_key("t", r#"{"a":1,"b":{"y":2,"x":[1,{"q":0,"p":1}]}}"#),
        function_key(
            "t",
            r#" { "b" : { "x" : [1, {"p":1,"q":0}], "y" : 2 }, "a" : 1 } "#
        ),
    );
    assert_ne!(
        function_key("t", r#"{"a":1}"#),
        function_key("t", r#"{"a":2}"#)
    );
    assert_eq!(
        ToolCallKey::new(
            &ToolName::plain("t"),
            &ToolPayload::Function {
                arguments: "{}".to_string()
            }
        ),
        ToolCallKey::new(
            &ToolName::namespaced("functions", "t"),
            &ToolPayload::Function {
                arguments: "{}".to_string()
            }
        ),
    );
}

#[test]
fn changed_arguments_are_not_blocked() {
    let failures = RepeatedToolFailures::default();
    let key = function_key("read", r#"{"path":"a"}"#);
    fail_times(&failures, &key, "not found", MAX_IDENTICAL_TOOL_FAILURES);

    let other = function_key("read", r#"{"path":"b"}"#);
    assert_eq!(None, failures.refusal(&other));
    // The original call stays refused until a different call completes.
    assert!(failures.refusal(&key).is_some());

    failures.record(other, Some("not found".to_string()));
    assert_eq!(None, failures.refusal(&key));
}

#[test]
fn success_resets_the_streak() {
    let failures = RepeatedToolFailures::default();
    let key = function_key("flaky", "{}");
    fail_times(&failures, &key, "boom", MAX_IDENTICAL_TOOL_FAILURES - 1);
    failures.record(key.clone(), None);
    fail_times(&failures, &key, "boom", MAX_IDENTICAL_TOOL_FAILURES - 1);
    assert_eq!(None, failures.refusal(&key));
}

#[test]
fn different_errors_are_not_deterministic_failures() {
    let failures = RepeatedToolFailures::default();
    let key = function_key("poll", "{}");
    for attempt in 0..(MAX_IDENTICAL_TOOL_FAILURES * 2) {
        assert_eq!(None, failures.refusal(&key));
        failures.record(key.clone(), Some(format!("attempt {attempt} failed")));
    }
    assert_eq!(None, failures.refusal(&key));
}

#[test]
fn failed_output_text_reads_only_failed_outputs() {
    let output = |success| ResponseItem::FunctionCallOutput {
        id: None,
        call_id: Some("call".to_string()),
        name: None,
        namespace: None,
        output: FunctionCallOutputPayload {
            body: FunctionCallOutputBody::Text(NO_GOAL.to_string()),
            success,
        },
        internal_chat_message_metadata_passthrough: None,
    };
    assert_eq!(
        Some(NO_GOAL.to_string()),
        failed_output_text(&output(Some(false)))
    );
    assert_eq!(None, failed_output_text(&output(Some(true))));
    assert_eq!(None, failed_output_text(&output(None)));
}
