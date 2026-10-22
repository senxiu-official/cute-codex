//! Stops a turn from looping on a tool call that keeps failing the same way.
//!
//! Some models answer a deterministic tool error by issuing the identical call
//! again, indefinitely. Once the same call (same tool, equivalent arguments)
//! has failed [`MAX_IDENTICAL_TOOL_FAILURES`] times in a row with the same
//! error, further repeats are not executed: the model gets an explanation
//! instead, and after [`MAX_REFUSED_REPEATS`] such refusals the turn ends.
//! Any success or any different call resets the streak, so ordinary retries
//! with changed arguments are unaffected. State is per turn.

use std::sync::Mutex;
use std::sync::PoisonError;

use codex_protocol::models::ResponseItem;
use codex_tools::ToolName;
use codex_tools::ToolPayload;
use serde_json::Value;

/// Identical failures allowed before repeats of the call are refused.
pub(crate) const MAX_IDENTICAL_TOOL_FAILURES: u32 = 3;
/// Refused repeats tolerated before the turn is stopped.
pub(crate) const MAX_REFUSED_REPEATS: u32 = 2;
/// Bound on how much of the error text is quoted back.
const MAX_QUOTED_ERROR_CHARS: usize = 500;

/// A tool call identity that ignores JSON key order and whitespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ToolCallKey {
    tool_name: ToolName,
    arguments: String,
}

impl ToolCallKey {
    pub(crate) fn new(tool_name: &ToolName, payload: &ToolPayload) -> Self {
        let arguments = match payload {
            ToolPayload::Function { arguments } => canonical_json(arguments),
            ToolPayload::Custom { input } => input.clone(),
            ToolPayload::ToolSearch { .. } => payload.log_payload().into_owned(),
        };
        Self {
            tool_name: tool_name.clone().with_default_namespace(),
            arguments,
        }
    }
}

fn canonical_json(arguments: &str) -> String {
    match serde_json::from_str::<Value>(arguments) {
        Ok(value) => sort_object_keys(value).to_string(),
        Err(_) => arguments.trim().to_string(),
    }
}

fn sort_object_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries = map
                .into_iter()
                .map(|(key, value)| (key, sort_object_keys(value)))
                .collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            Value::Object(entries.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_object_keys).collect()),
        other => other,
    }
}

/// Returns the error text when a tool output reports failure.
pub(crate) fn failed_output_text(item: &ResponseItem) -> Option<String> {
    let output = match item {
        ResponseItem::FunctionCallOutput { output, .. }
        | ResponseItem::CustomToolCallOutput { output, .. } => output,
        _ => return None,
    };
    (output.success == Some(false)).then(|| output.body.to_text().unwrap_or_default())
}

/// Per-turn record of the latest run of identical failing calls.
#[derive(Debug, Default)]
pub(crate) struct RepeatedToolFailures {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    streak: Option<FailureStreak>,
    stop_request: Option<String>,
}

#[derive(Debug)]
struct FailureStreak {
    key: ToolCallKey,
    error: String,
    failures: u32,
    refusals: u32,
}

impl RepeatedToolFailures {
    /// Decides, when a call is accepted, whether it may run. Returns the tool output
    /// to send instead when it may not. Too many refusals also request a turn stop,
    /// which the sampling loop collects with [`Self::take_stop_request`].
    pub(crate) fn refusal(&self, key: &ToolCallKey) -> Option<String> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let streak = state.streak.as_mut().filter(|streak| {
            streak.key == *key && streak.failures >= MAX_IDENTICAL_TOOL_FAILURES
        })?;
        streak.refusals = streak.refusals.saturating_add(1);
        let refusals = streak.refusals;
        let tool_name = streak.key.tool_name.clone();
        let failures = streak.failures;
        let error = quoted_error(&streak.error);
        let refusal = format!(
            "This exact `{tool_name}` call was not run: it already failed {failures} times in a row with the same error, so repeating it cannot succeed. Do not call it again with these arguments. Change the arguments, take a different approach, or explain the problem to the user. Last error: {error}"
        );
        if refusals > MAX_REFUSED_REPEATS && state.stop_request.is_none() {
            state.stop_request = Some(format!(
                "Stopped this turn because the model kept repeating a `{tool_name}` call that already failed {failures} times in a row with the same error: {error}"
            ));
        }
        Some(refusal)
    }

    /// Records the outcome of an executed call; `error` is `None` on success.
    pub(crate) fn record(&self, key: ToolCallKey, error: Option<String>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(error) = error else {
            state.streak = None;
            return;
        };
        match state.streak.as_mut() {
            Some(current) if current.key == key && current.error == error => {
                current.failures = current.failures.saturating_add(1);
            }
            _ => {
                state.streak = Some(FailureStreak {
                    key,
                    error,
                    failures: 1,
                    refusals: 0,
                });
            }
        }
    }

    /// Returns the explanation once when refused repeats should end the turn.
    pub(crate) fn take_stop_request(&self) -> Option<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stop_request
            .take()
    }
}

fn quoted_error(error: &str) -> String {
    let error = error.trim();
    match error.char_indices().nth(MAX_QUOTED_ERROR_CHARS) {
        Some((end, _)) => format!("{}...", &error[..end]),
        None => error.to_string(),
    }
}

#[cfg(test)]
#[path = "repeated_failures_tests.rs"]
mod tests;
