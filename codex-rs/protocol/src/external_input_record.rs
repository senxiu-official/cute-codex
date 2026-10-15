//! Model-invisible, versioned ExternalInput history facts.
use crate::external_input::Commit;
use crate::external_input::HoldReason;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Record {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub fact: Fact,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum Fact {
    Commit {
        commit: Box<Commit>,
    },
    Claim {
        key: MessageKey,
        #[serde(rename = "attemptId")]
        #[schemars(with = "String")]
        attempt_id: Uuid,
    },
    Output {
        key: MessageKey,
        #[serde(rename = "attemptId")]
        #[schemars(with = "String")]
        attempt_id: Uuid,
    },
    Hold {
        key: MessageKey,
        #[serde(rename = "attemptId")]
        #[schemars(with = "Option<String>")]
        attempt_id: Option<Uuid>,
        reason: HoldReason,
    },
    Retry {
        key: MessageKey,
        #[serde(rename = "expectedAttemptId")]
        #[schemars(with = "Option<String>")]
        expected_attempt_id: Option<Uuid>,
        #[serde(rename = "retryId")]
        retry_id: String,
    },
    DispatchGate {
        paused: bool,
        reason: GateReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "ExternalInputMessageKey", export_to = "v2/")]
#[schemars(rename = "ExternalInputMessageKey")]
pub struct MessageKey {
    pub message_id: String,
    pub semantic_sha256: String,
}

/// The private v1 gate represents an explicit interruption only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateReason {
    Interrupted,
}
