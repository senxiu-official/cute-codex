use super::session::Session;
use anyhow::Result;
use anyhow::ensure;
use codex_protocol::external_input::Processing;
use codex_protocol::external_input_record::Fact;
use codex_protocol::external_input_record::MessageKey;
use codex_protocol::external_input_record::Record;
use codex_protocol::turn_input::TurnInput;
use codex_protocol::turn_input::TurnInputMode;
use codex_protocol::turn_input::TurnInputRequest;
use codex_protocol::turn_input::TurnInputSubmission;
use std::sync::Arc;
use uuid::Uuid;

impl Session {
    pub(crate) async fn has_external_input_work(&self) -> bool {
        let state = self.external_input.lock().await;
        let Some(runtime) = state.as_ref() else {
            return false;
        };
        runtime.has_dispatch_work()
    }

    /// Called while admission is serialized by the external-input runtime lock.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "regular task identity and mailbox phase must be captured atomically"
    )]
    pub(crate) async fn external_input_soon_boundary(
        &self,
        turn: &super::turn_context::TurnContext,
        input: crate::external_input::InputBoundary,
    ) -> Option<crate::external_input::SoonBoundary> {
        let active = self.active_turn.lock().await;
        let active = active.as_ref()?;
        let task = active.task.as_ref()?;
        if task.kind != crate::state::TaskKind::Regular || task.turn_context.sub_id != turn.sub_id {
            return None;
        }
        let accepts_mail = active
            .turn_state
            .lock()
            .await
            .accepts_mailbox_delivery_for_current_turn();
        Some(crate::external_input::SoonBoundary {
            current: Arc::clone(&active.turn_state),
            input,
            accepts_mail,
        })
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "continuation eligibility must serialize against admission and the interruption gate"
    )]
    pub(crate) async fn has_external_input_soon_for_turn(
        &self,
        turn: &super::turn_context::TurnContext,
    ) -> bool {
        let state = self.external_input.lock().await;
        let Some(runtime) = state.as_ref() else {
            return false;
        };
        self.external_input_soon_boundary(turn, crate::external_input::InputBoundary::Drain)
            .await
            .is_some_and(|boundary| runtime.has_soon_continuation(&boundary))
    }

    /// Runs after upstream mailbox/user-queue lifecycle arbitration. The common
    /// pending table is deliberately outside the trigger mailbox checked by Core.
    pub(crate) fn dispatch_external_input(
        self: &Arc<Self>,
    ) -> futures::future::BoxFuture<'static, ()> {
        let session = Arc::clone(self);
        Box::pin(async move {
            if !session.has_external_input_work().await {
                return;
            }
            let revision = {
                let mut state = session.external_input.lock().await;
                let Some(runtime) = state.as_mut() else {
                    return;
                };
                if runtime.dispatch_revision == runtime.attempted_revision {
                    return;
                }
                runtime.attempted_revision = runtime.dispatch_revision;
                runtime.dispatch_revision
            };
            let result = super::turn_input::handle(
                &session,
                TurnInputRequest::new(TurnInput::ExternalInput),
                TurnInputMode::StartIfIdle,
                Uuid::now_v7().to_string(),
            )
            .await;
            // Rejection made no turn. Let a later lifecycle boundary arbitrate
            // again; a started/failed turn never grants itself another attempt.
            if matches!(result, Ok(TurnInputSubmission::NotSubmitted { .. })) {
                let mut state = session.external_input.lock().await;
                if let Some(runtime) = state.as_mut()
                    && runtime.attempted_revision == revision
                {
                    runtime.attempted_revision = revision - 1;
                }
            }
        })
    }

    /// Returns true only for a newly persisted permit. Exact replay does not wake.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "retry compare-and-set and its durable permit must serialize against claims and interruption"
    )]
    pub(crate) async fn retry_external_input(
        &self,
        owner: &str,
        key: MessageKey,
        expected: Option<Uuid>,
        retry_id: &str,
    ) -> Result<bool> {
        ensure!(
            !retry_id.is_empty() && retry_id.len() <= 256,
            "invalid retry id"
        );
        let excluded_turn = self
            .active_turn
            .lock()
            .await
            .as_ref()
            .map(|turn| Arc::clone(&turn.turn_state));
        let mut state = self.external_input.lock().await;
        let runtime = state
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("external input disabled"))?;
        ensure!(
            runtime.owner == owner && !runtime.poisoned,
            "external input recovery required"
        );
        let record = Record {
            version: 1,
            owner_id: owner.into(),
            thread_id: self.thread_id.to_string(),
            fact: Fact::Retry {
                key: key.clone(),
                expected_attempt_id: expected,
                retry_id: retry_id.into(),
            },
        };
        if let Some(previous) = runtime.retries.get(retry_id) {
            ensure!(previous == &record, "external input retry conflict");
            return Ok(false);
        }
        ensure!(
            runtime.policy_blocked.is_empty(),
            "receiver canonical byte policy blocks retry; adjust trusted launch policy and restart"
        );
        let found = runtime
            .recovery
            .messages
            .get_mut(&key.message_id)
            .ok_or_else(|| anyhow::anyhow!("retry requires existing A4 receipt"))?;
        ensure!(
            found.commit.envelope.semantic_sha256 == key.semantic_sha256,
            "external input conflict"
        );
        ensure!(
            found.commit.envelope.message.delivery.is_active(),
            "passive input cannot be retried"
        );
        let attempt = match found.processing {
            Processing::Pending(attempt) | Processing::Held(attempt, _) => attempt,
            _ => anyhow::bail!("claimed or completed input cannot be retried"),
        };
        ensure!(
            attempt == expected,
            "external input retry compare-and-set conflict"
        );
        let revision = runtime
            .dispatch_revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("external input dispatch revision exhausted"))?;
        runtime.poisoned = true;
        self.write_external_input_fact(owner, record.fact.clone())
            .await?;
        found.processing = Processing::Pending(expected);
        let _ = runtime.changed.send(key.message_id.clone());
        runtime.dispatch_revision = revision;
        if let Some(turn) = excluded_turn {
            runtime
                .retry_exclusions
                .insert(key.message_id.clone(), turn);
        }
        runtime.permits.insert(key.message_id);
        runtime.retries.insert(retry_id.into(), record);
        runtime.poisoned = false;
        Ok(true)
    }
}
