//! Canonical external data, deliberately without a chat role.
//!
//! S6b2 Director decision grants a task-scoped exception to AGENTS rule 6:
//! the existing message fragment trait cannot represent a standalone output.
//! Keep this typed sibling local to ExternalInput rather than changing that trait.
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Error;
use codex_protocol::models::ResponseItem;

use super::CanonicalBytePolicy;

/// A transport-valid envelope is not sufficient for model admission. This type
/// bounds the entire serialized item, including escaped JSON and identifiers.
/// The Human-approved receiver byte policy is independent of model/provider.
/// It makes no local or remote token-count guarantee. P0 context review applies.
pub(crate) struct ExternalInputContext {
    item: ResponseItem,
}

impl ExternalInputContext {
    pub(crate) fn new(envelope: &Envelope, policy: CanonicalBytePolicy) -> Result<Self, Error> {
        let item = envelope.response_item()?;
        let serialized =
            serde_json::to_vec(&item).map_err(|_| Error::Invalid("model item serialization"))?;
        if !policy.permits(serialized.len()) {
            return Err(Error::Invalid("receiver canonical byte policy exceeded"));
        }
        Ok(Self { item })
    }

    pub(crate) fn into_response_item(self) -> ResponseItem {
        self.item
    }
}

#[cfg(test)]
#[path = "external_input_tests.rs"]
mod tests;
