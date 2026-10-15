use crate::external_input::HoldReason;
use crate::external_input::Processing;
use crate::external_input::Receipt;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExternalInputDeliveryState", export_to = "v2/")]
#[schemars(rename = "ExternalInputDeliveryState")]
pub enum DeliveryState {
    Unknown,
    Pending,
    ContextPersisted,
    Conflict,
    RetryableError,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExternalInputProcessingState", export_to = "v2/")]
#[schemars(rename = "ExternalInputProcessingState")]
pub enum ProcessingState {
    None,
    Pending,
    Claimed,
    OutputObserved,
    Held,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "ExternalInputProcessingStatus", export_to = "v2/")]
#[schemars(rename = "ExternalInputProcessingStatus")]
pub struct ProcessingStatus {
    pub state: ProcessingState,
    #[schemars(with = "Option<String>")]
    pub attempt_id: Option<Uuid>,
    pub reason: Option<HoldReason>,
}

impl From<&Processing> for ProcessingStatus {
    fn from(value: &Processing) -> Self {
        let (state, attempt_id, reason) = match value {
            Processing::None => (ProcessingState::None, None, None),
            Processing::Pending(id) => (ProcessingState::Pending, *id, None),
            Processing::Claimed(id) => (ProcessingState::Claimed, Some(*id), None),
            Processing::OutputObserved(id) => (ProcessingState::OutputObserved, Some(*id), None),
            Processing::Held(id, reason) => (ProcessingState::Held, *id, Some(reason.clone())),
        };
        Self {
            state,
            attempt_id,
            reason,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "ExternalInputStatus", export_to = "v2/")]
#[schemars(rename = "ExternalInputStatus")]
pub struct Status {
    pub message_id: String,
    pub semantic_sha256: String,
    pub delivery_state: DeliveryState,
    pub receipt: Option<Receipt>,
    pub processing: ProcessingStatus,
}

/// Why exclusive admission refused an item. A refused item is never queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExternalInputBusyReason", export_to = "v2/")]
#[schemars(rename = "ExternalInputBusyReason")]
pub enum BusyReason {
    /// A turn is running or reserved.
    ActiveTurn,
    /// A committed active item is waiting for its processing turn.
    PendingObligation,
    /// Native mailbox input is waiting to start a turn.
    PendingTurnTrigger,
    /// The interruption gate is closed; see `thread/externalInput/resume`.
    GateClosed,
    /// Automatic turns do not run in Plan mode.
    PlanMode,
    /// The host is draining and admits no new turns.
    ServerDraining,
}
