//! Common ExternalInput envelopes, identities and strict pair recovery.
//! Structured view facts never enter the canonical model projection.
use crate::ResponseItemId;
use crate::models::FunctionCallOutputPayload;
use crate::models::ResponseItem;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("unsupported external input version")]
    Version,
    #[error("invalid external input: {0}")]
    Invalid(&'static str),
    #[error("external input identity conflict")]
    Conflict,
    #[error("partial or corrupt external input history")]
    Corrupt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExternalInputSourceKind", export_to = "v2/")]
#[schemars(rename = "ExternalInputSourceKind")]
pub enum SourceKind {
    Agent,
    Service,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ExternalInputSource", export_to = "v2/")]
#[schemars(rename = "ExternalInputSource")]
pub struct Source {
    pub kind: SourceKind,
    pub id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExternalInputDelivery", export_to = "v2/")]
#[schemars(rename = "ExternalInputDelivery")]
pub enum Delivery {
    AfterTurn,
    Passive,
    Soon,
}
impl Delivery {
    /// Active deliveries create one processing obligation after context persistence.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::AfterTurn | Self::Soon)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
#[ts(rename = "ExternalInputMessage", export_to = "v2/")]
#[schemars(rename = "ExternalInputMessage")]
pub struct Message {
    pub id: String,
    pub source: Source,
    #[serde(rename = "type")]
    pub event_type: String,
    pub delivery: Delivery,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "ExternalInputEnvelope", export_to = "v2/")]
#[schemars(rename = "ExternalInputEnvelope")]
pub struct Envelope {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub runtime_generation: u64,
    pub message: Message,
    pub semantic_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub view: Option<crate::external_input_view::View>,
}

fn bounded(value: &str, limit: usize) -> Result<(), Error> {
    if value.is_empty() || value.len() > limit {
        return Err(Error::Invalid("byte length"));
    }
    Ok(())
}
fn hash_fields(domain: &[u8], fields: &[&str]) -> Sha256 {
    let mut hash = Sha256::new();
    hash.update(domain);
    for field in fields {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    hash
}
impl Envelope {
    pub fn digest(&self) -> String {
        let fields: &[&str] = &[
            &self.owner_id,
            &self.thread_id,
            &self.message.id,
            match self.message.source.kind {
                SourceKind::Agent => "agent",
                SourceKind::Service => "service",
            },
            &self.message.source.id,
            &self.message.event_type,
            match self.message.delivery {
                Delivery::AfterTurn => "after_turn",
                Delivery::Soon => "soon",
                Delivery::Passive => "passive",
            },
            &self.message.text,
        ];
        let mut hash = hash_fields(
            if self.version == 2 {
                b"codex:external-input:v2\0"
            } else {
                b"codex:external-input:v1\0"
            },
            fields,
        );
        if self.version == 2 {
            let view = self.view.as_ref().map_or_else(
                || "null".to_string(),
                super::external_input_view::View::canonical_json,
            );
            hash.update((view.len() as u64).to_be_bytes());
            hash.update(view.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
    pub fn validate(&self) -> Result<(), Error> {
        if !matches!(self.version, 1 | 2) {
            return Err(Error::Version);
        }
        if self.version == 1 && self.view.is_some() {
            return Err(Error::Invalid("v1 view"));
        }
        if let Some(view) = &self.view {
            view.validate()?;
        }
        for value in [
            &self.owner_id,
            &self.thread_id,
            &self.message.id,
            &self.message.source.id,
            &self.message.event_type,
        ] {
            bounded(value, 256)?;
        }
        bounded(&self.message.text, 64 * 1024)?;
        if self.semantic_sha256 != self.digest() {
            return Err(Error::Invalid("semantic digest"));
        }
        Ok(())
    }
    /// Matching data is not authentication; the next caller must own the trusted binding.
    pub fn validate_binding(
        &self,
        owner: &str,
        thread: &str,
        generation: u64,
    ) -> Result<(), Error> {
        self.validate()?;
        if self.owner_id != owner
            || self.thread_id != thread
            || self.runtime_generation != generation
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    /// Only source/type/text enter model text. Mechanical identities and delivery do not.
    pub fn response_item(&self) -> Result<ResponseItem, Error> {
        self.validate()?;
        #[derive(Serialize)]
        struct Body<'a> {
            source: &'a Source,
            #[serde(rename = "type")]
            event_type: &'a str,
            text: &'a str,
        }
        let text = serde_json::to_string(&Body {
            source: &self.message.source,
            event_type: &self.message.event_type,
            text: &self.message.text,
        })
        .map_err(|_| Error::Invalid("body serialization"))?;
        Ok(ResponseItem::FunctionCallOutput {
            id: Some(ResponseItemId::from_server(self.message.id.clone())),
            call_id: None,
            name: Some("external_event".to_string()),
            namespace: Some("external".to_string()),
            output: FunctionCallOutputPayload::from_text(text),
            internal_chat_message_metadata_passthrough: None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "ExternalInputReceipt", export_to = "v2/")]
#[schemars(rename = "ExternalInputReceipt")]
pub struct Receipt {
    pub schema: String,
    pub receipt_id: String,
    pub owner_id: String,
    pub thread_id: String,
    pub message_id: String,
    pub semantic_sha256: String,
    pub response_item_id: String,
    pub turn_id: String,
    pub ordinal: u64,
}
impl Receipt {
    pub fn new(envelope: &Envelope, turn_id: String, ordinal: u64) -> Result<Self, Error> {
        envelope.validate()?;
        bounded(&turn_id, 256)?;
        if ordinal == 0 {
            return Err(Error::Invalid("ordinal"));
        }
        let mut receipt = Self {
            schema: "codex.external-input-receipt.v1".to_string(),
            receipt_id: String::new(),
            owner_id: envelope.owner_id.clone(),
            thread_id: envelope.thread_id.clone(),
            message_id: envelope.message.id.clone(),
            semantic_sha256: envelope.semantic_sha256.clone(),
            response_item_id: envelope.message.id.clone(),
            turn_id,
            ordinal,
        };
        let mut hash = hash_fields(
            b"codex:external-input-receipt:v1\0",
            &[
                &receipt.owner_id,
                &receipt.thread_id,
                &receipt.message_id,
                &receipt.semantic_sha256,
                &receipt.response_item_id,
                &receipt.turn_id,
            ],
        );
        hash.update(ordinal.to_be_bytes());
        receipt.receipt_id = format!("eir1_{:x}", hash.finalize());
        Ok(receipt)
    }
}

/// Mechanical records are deliberately separate from ResponseItem and RolloutItem.
/// The later writer must persist Commit immediately followed by its canonical item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Commit {
    pub envelope: Envelope,
    pub receipt: Receipt,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename = "ExternalInputHoldReason", export_to = "v2/")]
#[schemars(rename = "ExternalInputHoldReason")]
pub enum HoldReason {
    CanonicalSizePolicy,
    PlanMode,
    Interrupted,
    RequestUncertain,
    NoOutput,
    ContextMissing,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProcessingPhase {
    Claim {
        #[serde(rename = "attemptId")]
        #[schemars(with = "String")]
        attempt_id: Uuid,
    },
    Output {
        #[serde(rename = "attemptId")]
        #[schemars(with = "String")]
        attempt_id: Uuid,
    },
    Hold {
        #[serde(rename = "attemptId")]
        #[schemars(with = "Option<String>")]
        attempt_id: Option<Uuid>,
        reason: HoldReason,
    },
    Retry {
        #[serde(rename = "expectedAttemptId")]
        #[schemars(with = "Option<String>")]
        expected_attempt_id: Option<Uuid>,
        #[serde(rename = "retryId")]
        retry_id: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessingRecord {
    pub version: u32,
    pub owner_id: String,
    pub thread_id: String,
    pub message_id: String,
    pub semantic_sha256: String,
    pub processing: ProcessingPhase,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Processing {
    None,
    Pending(Option<Uuid>),
    Claimed(Uuid),
    OutputObserved(Uuid),
    Held(Option<Uuid>, HoldReason),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovered {
    pub commit: Commit,
    pub processing: Processing,
}
/// The adapter must preserve every intervening record as Other, never filter gaps.
/// Item turn IDs must come from the native containing turn, not the receipt.
/// Malformed records/read errors must fail before invoking recovery.
pub enum HistoryEntry<'a> {
    Commit(&'a Commit),
    Item {
        item: &'a ResponseItem,
        turn_id: &'a str,
    },
    Processing(&'a ProcessingRecord),
    Other,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Recovery {
    owner_id: String,
    thread_id: String,
    pub messages: BTreeMap<String, Recovered>,
    pub next_ordinal: u64,
}
impl Recovery {
    /// None means absent only after successful recovery of the complete history.
    pub fn lookup(&self, envelope: &Envelope) -> Result<Option<&Recovered>, Error> {
        envelope.validate()?;
        if envelope.owner_id != self.owner_id || envelope.thread_id != self.thread_id {
            return Err(Error::Conflict);
        }
        match self.messages.get(&envelope.message.id) {
            Some(found)
                if found.commit.envelope.owner_id != envelope.owner_id
                    || found.commit.envelope.thread_id != envelope.thread_id
                    || found.commit.envelope.semantic_sha256 != envelope.semantic_sha256 =>
            {
                Err(Error::Conflict)
            }
            found => Ok(found),
        }
    }
}

/// Validate records, not permission to dispatch. A persisted retry can follow an
/// unfinished claim after recovery held that claim as request_uncertain; the later
/// writer must authorize the retry before appending it. No retry is inferred here.
pub fn recover(owner: &str, thread: &str, history: &[HistoryEntry<'_>]) -> Result<Recovery, Error> {
    let mut result = Recovery {
        owner_id: owner.to_string(),
        thread_id: thread.to_string(),
        messages: BTreeMap::new(),
        next_ordinal: 1,
    };
    let mut retries = BTreeMap::new();
    let mut attempts = std::collections::BTreeSet::new();
    let mut index = 0;
    while index < history.len() {
        match &history[index] {
            HistoryEntry::Commit(commit) => {
                let envelope = &commit.envelope;
                envelope.validate()?;
                if envelope.owner_id != owner || envelope.thread_id != thread {
                    return Err(Error::Conflict);
                }
                if commit.receipt.schema != "codex.external-input-receipt.v1" {
                    return Err(Error::Version);
                }
                if commit.receipt
                    != Receipt::new(
                        envelope,
                        commit.receipt.turn_id.clone(),
                        commit.receipt.ordinal,
                    )?
                {
                    return Err(Error::Corrupt);
                }
                let Some(HistoryEntry::Item { item, turn_id }) = history.get(index + 1) else {
                    return Err(Error::Corrupt);
                };
                if **item != envelope.response_item()? || *turn_id != commit.receipt.turn_id {
                    return Err(Error::Corrupt);
                }
                if result.messages.contains_key(&envelope.message.id) {
                    return Err(Error::Conflict);
                }
                if commit.receipt.ordinal != result.next_ordinal {
                    return Err(Error::Corrupt);
                }
                result.next_ordinal = result.next_ordinal.checked_add(1).ok_or(Error::Corrupt)?;
                result.messages.insert(
                    envelope.message.id.clone(),
                    Recovered {
                        commit: (*commit).clone(),
                        processing: match envelope.message.delivery {
                            Delivery::AfterTurn | Delivery::Soon => Processing::Pending(None),
                            Delivery::Passive => Processing::None,
                        },
                    },
                );
                index += 1;
            }
            HistoryEntry::Item {
                item:
                    ResponseItem::FunctionCallOutput {
                        name, namespace, ..
                    },
                ..
            } if name.as_deref() == Some("external_event")
                && namespace.as_deref() == Some("external") =>
            {
                return Err(Error::Corrupt);
            }
            HistoryEntry::Processing(record) => {
                if record.version != 1 {
                    return Err(Error::Version);
                }
                if record.owner_id != owner || record.thread_id != thread {
                    return Err(Error::Conflict);
                }
                let found = result
                    .messages
                    .get_mut(&record.message_id)
                    .ok_or(Error::Corrupt)?;
                if found.commit.envelope.semantic_sha256 != record.semantic_sha256 {
                    return Err(Error::Conflict);
                }
                let current_attempt = match &found.processing {
                    Processing::Claimed(id) | Processing::OutputObserved(id) => Some(*id),
                    Processing::Held(id, _) | Processing::Pending(id) => *id,
                    Processing::None => None,
                };
                found.processing = match &record.processing {
                    ProcessingPhase::Claim { attempt_id }
                        if matches!(found.processing, Processing::Pending(_))
                            && !attempt_id.is_nil()
                            && attempts.insert(*attempt_id) =>
                    {
                        Processing::Claimed(*attempt_id)
                    }
                    ProcessingPhase::Output { attempt_id }
                        if found.processing == Processing::Claimed(*attempt_id) =>
                    {
                        Processing::OutputObserved(*attempt_id)
                    }
                    ProcessingPhase::Hold { attempt_id, reason }
                        if *attempt_id == current_attempt
                            && matches!(
                                found.processing,
                                Processing::Pending(_) | Processing::Claimed(_)
                            ) =>
                    {
                        if matches!(reason, HoldReason::RequestUncertain | HoldReason::NoOutput)
                            && attempt_id.is_none()
                        {
                            return Err(Error::Corrupt);
                        }
                        Processing::Held(*attempt_id, reason.clone())
                    }
                    ProcessingPhase::Retry {
                        expected_attempt_id,
                        retry_id,
                    } => {
                        bounded(retry_id, 256)?;
                        if let Some(previous) = retries.get(retry_id) {
                            if *previous != **record {
                                return Err(Error::Conflict);
                            }
                            index += 1;
                            continue;
                        }
                        if !matches!(
                            found.processing,
                            Processing::Pending(_)
                                | Processing::Held(_, _)
                                | Processing::Claimed(_)
                        ) || *expected_attempt_id != current_attempt
                        {
                            return Err(Error::Corrupt);
                        }
                        retries.insert(retry_id.clone(), (*record).clone());
                        Processing::Pending(*expected_attempt_id)
                    }
                    _ => return Err(Error::Corrupt),
                };
            }
            HistoryEntry::Item { .. } | HistoryEntry::Other => {}
        }
        index += 1;
    }
    for found in result.messages.values_mut() {
        if let Processing::Claimed(id) = found.processing {
            found.processing = Processing::Held(Some(id), HoldReason::RequestUncertain);
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "external_input_tests.rs"]
mod tests;
