use super::Pending;
use super::Runtime;
use crate::state::TurnState;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Processing;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputBoundary {
    /// Fresh direct input: only admission before task installation may join.
    Initial,
    /// Upstream permits pending input to join the next request.
    Drain,
    /// Resume model/tool continuation after compaction before new pending input.
    Deferred,
}

pub(crate) struct SoonBoundary {
    pub current: Arc<Mutex<TurnState>>,
    pub input: InputBoundary,
    pub accepts_mail: bool,
}

impl Runtime {
    pub(crate) fn can_consume_soon(&self, pending: &Pending, boundary: &SoonBoundary) -> bool {
        !self.poisoned
            && self.policy_blocked.is_empty()
            && !self.paused
            && pending.envelope.message.delivery == Delivery::Soon
            && pending.exclusive_turn.is_none()
            && !self.excluded_by_exclusive_turn(pending, Some(&boundary.current))
            && boundary.accepts_mail
            && match boundary.input {
                InputBoundary::Drain => true,
                InputBoundary::Initial => !pending
                    .running_turn
                    .as_ref()
                    .is_some_and(|running| Arc::ptr_eq(running, &boundary.current)),
                InputBoundary::Deferred => false,
            }
    }

    pub(crate) fn has_soon_continuation(&self, boundary: &SoonBoundary) -> bool {
        self.pending
            .iter()
            .any(|pending| self.can_consume_soon(pending, boundary))
            || (!self.poisoned
                && self.policy_blocked.is_empty()
                && boundary.accepts_mail
                && boundary.input == InputBoundary::Drain
                && self.recovery.messages.iter().any(|(id, found)| {
                    found.commit.envelope.message.delivery == Delivery::Soon
                        && matches!(found.processing, Processing::Pending(_))
                        && (!self.paused || self.permits.contains(id))
                        && !self
                            .retry_exclusions
                            .get(id)
                            .is_some_and(|excluded| Arc::ptr_eq(excluded, &boundary.current))
                }))
    }
}

#[cfg(test)]
#[path = "eligibility_tests.rs"]
mod tests;
