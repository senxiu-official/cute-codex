//! Private, common ExternalInput runtime state. Admission is volatile; only
//! committed context and processing facts belong to native history.
mod api;
mod eligibility;
pub(crate) use eligibility::InputBoundary;
pub(crate) use eligibility::SoonBoundary;
#[cfg(all(debug_assertions, unix))]
pub(crate) mod probe;
pub(crate) mod recovery;

use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Recovery;
use codex_protocol::external_input_record::Record;
use std::collections::BTreeMap;

pub(crate) struct Pending {
    pub envelope: Envelope,
    /// Soon arriving after task installation must wait for an input drain.
    pub running_turn: Option<std::sync::Arc<tokio::sync::Mutex<crate::state::TurnState>>>,
    /// Identity of the active/reserved TurnState at after_turn admission.
    pub excluded_turn: Option<std::sync::Arc<tokio::sync::Mutex<crate::state::TurnState>>>,
    /// Exclusive admission: only this reserved turn may consume the item.
    pub exclusive_turn: Option<ExclusiveTurn>,
}

/// A turn reserved by exclusive admission. It consumes its bound item and no
/// other active pending input. A stop_answer turn excludes passive input too.
#[derive(Clone)]
pub(crate) struct ExclusiveTurn {
    pub id: String,
    pub state: std::sync::Arc<tokio::sync::Mutex<crate::state::TurnState>>,
    /// Only this identity may pass the Stop gate, and only in this turn.
    pub stop_answer: Option<codex_protocol::external_input_record::MessageKey>,
}

pub(crate) struct Runtime {
    pub policy: crate::context::CanonicalBytePolicy,
    pub policy_blocked: std::collections::BTreeSet<String>,
    /// One automatic reservation per new admission/retry, including failed turns.
    pub dispatch_revision: u64,
    pub attempted_revision: u64,
    pub retry_exclusions:
        BTreeMap<String, std::sync::Arc<tokio::sync::Mutex<crate::state::TurnState>>>,
    pub changed: tokio::sync::broadcast::Sender<String>,
    pub owner: String,
    pub recovery: Recovery,
    pub pending: Vec<Pending>,
    /// Weak reservations cannot outlive refused, blocked or cancelled owner input.
    pub owner_carries: BTreeMap<String, std::sync::Weak<Vec<Envelope>>>,
    pub paused: bool,
    /// Volatile provenance: only Stop in this process can allow stop_answer.
    /// Ordinary pause and restored gates never acquire this permission.
    pub stopped: bool,
    /// Set before fallible writes. Cancellation and uncertain completion leave
    /// it set, preventing subsequent sampling until strict process recovery.
    pub poisoned: bool,
    pub retries: BTreeMap<String, Record>,
    pub permits: std::collections::BTreeSet<String>,
    /// The reserved or running exclusive turn, if any.
    pub exclusive_turn: Option<ExclusiveTurn>,
}

impl Runtime {
    pub(crate) fn has_dispatch_work(&self) -> bool {
        !self.poisoned
            && self.policy_blocked.is_empty()
            && ((!self.paused
                && self.pending.iter().any(|pending| {
                    pending.exclusive_turn.is_none()
                        && pending.envelope.message.delivery.is_active()
                }))
                || self.recovery.messages.iter().any(|(id, found)| {
                    matches!(
                        found.processing,
                        codex_protocol::external_input::Processing::Pending(_)
                    ) && (!self.paused || self.permits.contains(id))
                }))
    }

    /// True if `pending` must not join the turn whose state is `current`: a
    /// bound item joins only its own turn, and an exclusive turn takes no
    /// other active item.
    pub(crate) fn excluded_by_exclusive_turn(
        &self,
        pending: &Pending,
        current: Option<&std::sync::Arc<tokio::sync::Mutex<crate::state::TurnState>>>,
    ) -> bool {
        let is_current = |turn: &ExclusiveTurn| {
            current.is_some_and(|current| std::sync::Arc::ptr_eq(&turn.state, current))
        };
        match &pending.exclusive_turn {
            Some(bound) => !is_current(bound),
            None => {
                (pending.envelope.message.delivery.is_active()
                    || self
                        .exclusive_turn
                        .as_ref()
                        .is_some_and(|turn| turn.stop_answer.is_some()))
                    && self.exclusive_turn.as_ref().is_some_and(is_current)
            }
        }
    }

    /// Gate permission is scoped to the exact reserved turn and message, not a
    /// retry permit or a temporary global reopening.
    pub(crate) fn stop_answer_key(
        &self,
        current: Option<&std::sync::Arc<tokio::sync::Mutex<crate::state::TurnState>>>,
    ) -> Option<&codex_protocol::external_input_record::MessageKey> {
        self.exclusive_turn
            .as_ref()
            .filter(|turn| {
                current.is_some_and(|current| std::sync::Arc::ptr_eq(&turn.state, current))
            })
            .and_then(|turn| turn.stop_answer.as_ref())
    }

    pub(crate) fn processing_allowed(
        &self,
        id: &str,
        current: Option<&std::sync::Arc<tokio::sync::Mutex<crate::state::TurnState>>>,
    ) -> bool {
        if let Some(key) = self.stop_answer_key(current) {
            return key.message_id == id && (!self.paused || self.stopped);
        }
        !self.paused || self.permits.contains(id)
    }

    /// Pending admissions plus unfinished committed obligations.
    pub(crate) fn admission_load(&self) -> usize {
        self.pending.len()
            + self
                .owner_carries
                .values()
                .filter(|entry| entry.strong_count() > 0)
                .count()
            + self
                .recovery
                .messages
                .values()
                .filter(|found| {
                    !matches!(
                        found.processing,
                        codex_protocol::external_input::Processing::None
                            | codex_protocol::external_input::Processing::OutputObserved(_)
                    )
                })
                .count()
    }
}
