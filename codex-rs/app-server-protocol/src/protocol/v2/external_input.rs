use crate::JsonSchema;
use crate::TS;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Message;
use codex_protocol::external_input_record::MessageKey;
use codex_protocol::external_input_status::Status;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputSubmitParams {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub message: Message,
    #[ts(optional = nullable)]
    pub view: Option<codex_protocol::external_input_view::View>,
    pub semantic_sha256: String,
    /// Absent means `queue`. Not part of the envelope or its digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub admission: Option<ExternalInputAdmission>,
}

/// How a submitted item may start or join a turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "v2/")]
pub enum ExternalInputAdmission {
    /// Pending until a safe input boundary; idle threads start an automatic turn.
    #[default]
    Queue,
    /// Start a dedicated turn now, or fail with `busy` and queue nothing. The
    /// turn consumes this item, passive input, and no other active item.
    /// Requires an active delivery.
    ExclusiveTurn,
}

impl From<ExternalInputSubmitParams> for Envelope {
    fn from(value: ExternalInputSubmitParams) -> Self {
        Self {
            version: value.version,
            owner_id: value.owner_id,
            thread_id: value.thread_id,
            runtime_generation: value.runtime_generation,
            message: value.message,
            view: value.view,
            semantic_sha256: value.semantic_sha256,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputStatusParams {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub messages: Vec<MessageKey>,
}

/// Submit returns this same shape with exactly one status; status preserves the
/// requested batch's count and order. Only context_persisted carries a receipt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputResponse {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub statuses: Vec<Status>,
    /// `exclusive_turn` submit only: the turn that carries this item. The
    /// receipt, once context is persisted, carries the same turn id; if the
    /// turn ends first, the item is dropped and its status becomes `unknown`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn_id: Option<String>,
}

/// Reopens the interruption gate without a human turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputResumeParams {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputResumeResponse {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    /// Whether the gate was closed before this call.
    pub reopened: bool,
}

/// Interrupts the named active turn without closing the interruption gate.
/// Responds like `turn/interrupt`, once the turn is aborted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputInterruptParams {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub turn_id: String,
}

/// Same wire shape as `TurnInterruptResponse`, sent once the turn is aborted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "v2/")]
pub struct ExternalInputInterruptResponse {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputRetryParams {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub message_id: String,
    pub semantic_sha256: String,
    #[schemars(with = "Option<String>")]
    pub expected_attempt_id: Option<Uuid>,
    pub retry_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "v2/")]
pub enum ExternalInputRetryDisposition {
    Released,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputRetryResponse {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub message_id: String,
    pub semantic_sha256: String,
    #[schemars(with = "Option<String>")]
    pub expected_attempt_id: Option<Uuid>,
    pub retry_id: String,
    pub disposition: ExternalInputRetryDisposition,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct ExternalInputStatusChangedNotification {
    pub thread_id: String,
    pub message_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientRequest;
    use crate::experimental_api::ExperimentalApi;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn submit(admission: Option<&str>) -> serde_json::Value {
        let mut params = json!({
            "version": 1,
            "ownerId": "owner",
            "threadId": "thread",
            "runtimeGeneration": 3,
            "message": {
                "id": "ask-1",
                "source": {"kind": "agent", "id": "requester"},
                "type": "ASK",
                "delivery": "after_turn",
                "text": "text",
            },
            "view": null,
            "semanticSha256": "0".repeat(64),
        });
        if let Some(admission) = admission {
            params["admission"] = json!(admission);
        }
        params
    }

    #[test]
    fn submit_admission_defaults_to_queue_and_stays_out_of_the_envelope() {
        let queued: ExternalInputSubmitParams = serde_json::from_value(submit(None)).unwrap();
        assert_eq!(queued.admission, None);
        assert_eq!(serde_json::to_value(&queued).unwrap(), submit(None));
        let exclusive: ExternalInputSubmitParams =
            serde_json::from_value(submit(Some("exclusive_turn"))).unwrap();
        assert_eq!(
            exclusive.admission,
            Some(ExternalInputAdmission::ExclusiveTurn)
        );
        assert_eq!(
            Envelope::from(exclusive),
            Envelope::from(queued),
            "admission is not part of the envelope"
        );
        assert!(
            serde_json::from_value::<ExternalInputSubmitParams>(submit(Some("exclusiveTurn")))
                .is_err()
        );
    }

    #[test]
    fn response_turn_id_is_optional() {
        let response = json!({
            "version": 1,
            "ownerId": "owner",
            "threadId": "thread",
            "runtimeGeneration": 3,
            "statuses": [],
        });
        let parsed: ExternalInputResponse = serde_json::from_value(response.clone()).unwrap();
        assert_eq!(parsed.turn_id, None);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), response);
        let mut started = response;
        started["turnId"] = json!("turn-1");
        let parsed: ExternalInputResponse = serde_json::from_value(started).unwrap();
        assert_eq!(parsed.turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn gate_controls_are_experimental_owner_requests() {
        let binding = json!({
            "version": 1,
            "ownerId": "owner",
            "threadId": "thread",
            "runtimeGeneration": 3,
        });
        let resume: ClientRequest = serde_json::from_value(json!({
            "id": 1,
            "method": "thread/externalInput/resume",
            "params": binding,
        }))
        .unwrap();
        assert_eq!(
            resume.experimental_reason(),
            Some("thread/externalInput/resume")
        );
        let mut params = binding;
        params["turnId"] = json!("turn-1");
        let interrupt: ClientRequest = serde_json::from_value(json!({
            "id": 2,
            "method": "thread/externalInput/interrupt",
            "params": params,
        }))
        .unwrap();
        assert_eq!(
            interrupt.experimental_reason(),
            Some("thread/externalInput/interrupt")
        );
        let ClientRequest::ThreadExternalInputInterrupt { params, .. } = interrupt else {
            panic!("expected interrupt request");
        };
        assert_eq!(params.turn_id, "turn-1");
    }
}
