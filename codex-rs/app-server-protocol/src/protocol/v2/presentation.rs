use crate::JsonSchema;
use crate::TS;
use codex_protocol::presentation::Presentation;
use codex_protocol::presentation::PresentationAppended;
use serde::Deserialize;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct PresentationAppendParams {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub presentation: Presentation,
    pub semantic_sha256: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct PresentationStatusParams {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub presentation_id: String,
    pub semantic_sha256: String,
}
/// A receipt confirms durable display data only, never A4, rendered or seen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct PresentationResponse {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub receipt: Option<PresentationAppended>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "v2/")]
pub struct PresentationAppendedNotification {
    pub thread_id: String,
    #[ts(type = "number")]
    pub position: u64,
    pub item: PresentationAppended,
}
