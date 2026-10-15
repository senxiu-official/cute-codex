//! Exclusive ExternalInput admission and gate control for the trusted launch owner.
//!
//! An exclusive item starts its own turn at once or is refused; it is never queued.
//! Its turn consumes that item and no other active pending input. Admission stays
//! volatile: if the turn ends before the durable pair is written, the item is
//! dropped and its status returns to unknown.
use super::external_input::snapshot;
use super::session::Session;
use crate::context::ExternalInputContext;
use crate::external_input::ExclusiveTurn;
use crate::external_input::Pending;
use crate::state::ActiveTurn;
use crate::state::TurnState;
use anyhow::Result;
use anyhow::ensure;
use codex_protocol::config_types::ModeKind;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Processing;
use codex_protocol::external_input_status::BusyReason;
use codex_protocol::external_input_status::DeliveryState;
use codex_protocol::external_input_status::Status;
use codex_protocol::turn_input::NotSubmittedReason;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[cfg(test)]
#[path = "external_input_exclusive_tests.rs"]
mod tests;

/// Status of the submitted item and the turn that carries (or carried) it.
pub(crate) type ExclusiveAdmission = (Status, Option<String>);

enum Reservation {
    Known(ExclusiveAdmission),
    Reserved(Status, Arc<Mutex<TurnState>>),
    Busy(BusyReason),
}

impl Session {
    pub(crate) async fn submit_exclusive_external_input(
        self: &Arc<Self>,
        envelope: Envelope,
    ) -> Result<std::result::Result<ExclusiveAdmission, BusyReason>> {
        envelope.validate()?;
        ensure!(
            envelope.thread_id == self.thread_id.to_string(),
            "external input thread mismatch"
        );
        ensure!(
            envelope.message.delivery.is_active(),
            "exclusive admission requires an active delivery"
        );
        // Same pre-checks as an automatic idle start, before reserving.
        if self.input_queue.has_trigger_turn_mailbox_items().await {
            return Ok(Err(BusyReason::PendingTurnTrigger));
        }
        if self.collaboration_mode().await.mode == ModeKind::Plan {
            return Ok(Err(BusyReason::PlanMode));
        }
        let Some(_admission) = self.services.extensions.admit_turn_start() else {
            return Ok(Err(BusyReason::ServerDraining));
        };
        let turn_id = Uuid::now_v7().to_string();
        let message_id = envelope.message.id.clone();
        let (status, turn_state) = match self
            .reserve_exclusive_external_input(envelope, &turn_id)
            .await?
        {
            Reservation::Known(known) => return Ok(Ok(known)),
            Reservation::Busy(reason) => return Ok(Err(reason)),
            Reservation::Reserved(status, turn_state) => (status, turn_state),
        };
        let started = super::turn_input::start_reserved_external_input_turn(
            self,
            &turn_state,
            turn_id.clone(),
        )
        .await;
        if !matches!(started, Ok(None)) {
            self.release_exclusive_external_input(&turn_state).await;
        }
        match started {
            Ok(None) => {}
            Ok(Some(NotSubmittedReason::PlanMode)) => return Ok(Err(BusyReason::PlanMode)),
            Ok(Some(_)) => return Ok(Err(BusyReason::PendingTurnTrigger)),
            Err(error) => return Err(error.into()),
        }
        // An interrupt can take a reservation before its task is installed; the
        // replacement turn never consumes the bound item, so release it now.
        let still_reserved = self
            .active_turn
            .lock()
            .await
            .as_ref()
            .is_some_and(|turn| Arc::ptr_eq(&turn.turn_state, &turn_state));
        if !still_reserved {
            self.release_exclusive_external_input(&turn_state).await;
        }
        tracing::debug!(%message_id, %turn_id, "exclusive external input turn started");
        Ok(Ok((status, Some(turn_id))))
    }

    /// Admission and idle reservation are one decision under the runtime lock,
    /// in the same lock order as ordinary admission and idle reservation.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "duplicate validation, the gate, obligations and the idle reservation must be atomic"
    )]
    async fn reserve_exclusive_external_input(
        &self,
        envelope: Envelope,
        turn_id: &str,
    ) -> Result<Reservation> {
        let mut state = self.external_input.lock().await;
        let runtime = state
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("external input disabled"))?;
        ensure!(
            runtime.owner == envelope.owner_id && !runtime.poisoned,
            "external input recovery required"
        );
        let id = envelope.message.id.clone();
        let digest = envelope.semantic_sha256.clone();
        let found = snapshot(runtime, &id, &digest);
        if found.delivery_state != DeliveryState::Unknown {
            // Exact replay never starts a second turn.
            let turn = found
                .receipt
                .as_ref()
                .map(|receipt| receipt.turn_id.clone())
                .or_else(|| {
                    runtime
                        .pending
                        .iter()
                        .find(|pending| pending.envelope.message.id == id)
                        .and_then(|pending| pending.exclusive_turn.as_ref())
                        .map(|turn| turn.id.clone())
                });
            return Ok(Reservation::Known((found, turn)));
        }
        ensure!(
            runtime.policy_blocked.is_empty(),
            "receiver canonical byte policy blocks restored history; adjust trusted launch policy and restart"
        );
        ExternalInputContext::new(&envelope, runtime.policy)?;
        ensure!(runtime.admission_load() < 100, "external input overloaded");
        if runtime.paused {
            return Ok(Reservation::Busy(BusyReason::GateClosed));
        }
        // Released committed obligations would be claimed by any next turn.
        if runtime
            .recovery
            .messages
            .values()
            .any(|found| matches!(found.processing, Processing::Pending(_)))
        {
            return Ok(Reservation::Busy(BusyReason::PendingObligation));
        }
        let mut active = self.active_turn.lock().await;
        if active.is_some() {
            return Ok(Reservation::Busy(BusyReason::ActiveTurn));
        }
        let turn_state = Arc::clone(&active.get_or_insert_with(ActiveTurn::default).turn_state);
        let bound = ExclusiveTurn {
            id: turn_id.to_string(),
            state: Arc::clone(&turn_state),
        };
        runtime.pending.push(Pending {
            envelope,
            running_turn: None,
            excluded_turn: None,
            exclusive_turn: Some(bound.clone()),
        });
        runtime.exclusive_turn = Some(bound);
        drop(active);
        Ok(Reservation::Reserved(
            snapshot(runtime, &id, &digest),
            turn_state,
        ))
    }

    /// Ends exclusivity for a finished, aborted or abandoned turn and drops its
    /// bound item if the turn never consumed it.
    pub(crate) async fn release_exclusive_external_input(&self, turn_state: &Mutex<TurnState>) {
        let mut state = self.external_input.lock().await;
        let Some(runtime) = state.as_mut() else {
            return;
        };
        let is_turn = |turn: &ExclusiveTurn| std::ptr::eq(Arc::as_ptr(&turn.state), turn_state);
        if runtime.exclusive_turn.as_ref().is_some_and(is_turn) {
            runtime.exclusive_turn = None;
        }
        let mut dropped = Vec::new();
        runtime.pending.retain(|pending| {
            let keep = !pending.exclusive_turn.as_ref().is_some_and(is_turn);
            if !keep {
                dropped.push(pending.envelope.message.id.clone());
            }
            keep
        });
        for id in dropped {
            tracing::warn!(message_id = %id, "exclusive external input turn ended before admission");
            let _ = runtime.changed.send(id);
        }
    }
}
