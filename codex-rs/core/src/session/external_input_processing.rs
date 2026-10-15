use super::session::Session;
use anyhow::Result;
use anyhow::ensure;
use codex_history::RolloutItem;
use codex_protocol::external_input::HoldReason;
use codex_protocol::external_input::Processing;
use codex_protocol::external_input_record::Fact;
use codex_protocol::external_input_record::MessageKey;
use codex_protocol::external_input_record::Record;
use codex_protocol::models::ResponseItem;
use uuid::Uuid;

impl Session {
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "canonical validation and durable claims must serialize against retry and interruption"
    )]
    pub(crate) async fn external_input_claim(
        &self,
        input: &[ResponseItem],
    ) -> Result<Vec<(String, Uuid)>> {
        let current_turn = self
            .active_turn
            .lock()
            .await
            .as_ref()
            .map(|turn| std::sync::Arc::clone(&turn.turn_state));
        let mut state = self.external_input.lock().await;
        let Some(runtime) = state.as_mut() else {
            return Ok(Vec::new());
        };
        ensure!(
            !runtime.poisoned,
            "external input recovery required before sampling"
        );
        ensure!(
            runtime.policy_blocked.is_empty(),
            "receiver canonical byte policy blocks restored history"
        );
        // Validate every eligible canonical item before persisting any claim.
        // Otherwise a missing later item could strand an earlier claim although
        // no request was attempted.
        let mut missing = Vec::new();
        for (id, found) in &runtime.recovery.messages {
            let excluded = runtime
                .retry_exclusions
                .get(id)
                .zip(current_turn.as_ref())
                .is_some_and(|(old, current)| std::sync::Arc::ptr_eq(old, current));
            if matches!(found.processing, Processing::Pending(_))
                && (!runtime.paused || runtime.permits.contains(id))
                && !excluded
                && !input.contains(&found.commit.envelope.response_item()?)
            {
                missing.push(id.clone());
            }
        }
        for id in &missing {
            let found = runtime
                .recovery
                .messages
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("validated external input disappeared"))?;
            let Processing::Pending(attempt) = found.processing else {
                unreachable!()
            };
            runtime.poisoned = true;
            self.write_external_input_fact(
                &runtime.owner,
                Fact::Hold {
                    key: MessageKey {
                        message_id: id.clone(),
                        semantic_sha256: found.commit.envelope.semantic_sha256.clone(),
                    },
                    attempt_id: attempt,
                    reason: HoldReason::ContextMissing,
                },
            )
            .await?;
            found.processing = Processing::Held(attempt, HoldReason::ContextMissing);
            runtime.permits.remove(id);
            let _ = runtime.changed.send(id.clone());
            runtime.poisoned = false;
        }
        ensure!(
            missing.is_empty(),
            "canonical external input missing from effective request"
        );
        let mut claims = Vec::new();
        for (id, found) in &mut runtime.recovery.messages {
            if !matches!(found.processing, Processing::Pending(_))
                || (runtime.paused && !runtime.permits.contains(id))
                || runtime
                    .retry_exclusions
                    .get(id)
                    .zip(current_turn.as_ref())
                    .is_some_and(|(old, current)| std::sync::Arc::ptr_eq(old, current))
            {
                continue;
            }
            let canonical = found.commit.envelope.response_item()?;
            let key = MessageKey {
                message_id: id.clone(),
                semantic_sha256: found.commit.envelope.semantic_sha256.clone(),
            };
            let (fact, processing) = if input.contains(&canonical) {
                let attempt = Uuid::now_v7();
                claims.push((id.clone(), attempt));
                (
                    Fact::Claim {
                        key,
                        attempt_id: attempt,
                    },
                    Processing::Claimed(attempt),
                )
            } else {
                let attempt = match found.processing {
                    Processing::Pending(id) => id,
                    _ => unreachable!(),
                };
                (
                    Fact::Hold {
                        key,
                        attempt_id: attempt,
                        reason: HoldReason::ContextMissing,
                    },
                    Processing::Held(attempt, HoldReason::ContextMissing),
                )
            };
            runtime.poisoned = true;
            self.write_external_input_fact(&runtime.owner, fact).await?;
            found.processing = processing;
            let _ = runtime.changed.send(id.clone());
            runtime.permits.remove(id);
            runtime.retry_exclusions.remove(id);
            runtime.poisoned = false;
            ensure!(
                !matches!(
                    found.processing,
                    Processing::Held(_, HoldReason::ContextMissing)
                ),
                "canonical external input missing from effective request"
            );
        }
        Ok(claims)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "durable outcome records and in-memory processing state must publish atomically"
    )]
    pub(crate) async fn finish_external_input_attempt(
        &self,
        claims: &[(String, Uuid)],
        outcome: Option<bool>,
    ) -> Result<()> {
        let mut state = self.external_input.lock().await;
        let Some(runtime) = state.as_mut() else {
            return Ok(());
        };
        ensure!(!runtime.poisoned, "external input recovery required");
        for (id, attempt) in claims {
            let found = runtime
                .recovery
                .messages
                .get_mut(id)
                .ok_or_else(|| anyhow::anyhow!("external input claim missing"))?;
            // A completed response is authoritative even if later tool drain,
            // accounting, or cancellation fails. Never downgrade its observation.
            if found.processing == Processing::OutputObserved(*attempt)
                || found.processing == Processing::Held(Some(*attempt), HoldReason::NoOutput)
            {
                continue;
            }
            ensure!(
                found.processing == Processing::Claimed(*attempt),
                "external input claim changed"
            );
            let key = MessageKey {
                message_id: id.clone(),
                semantic_sha256: found.commit.envelope.semantic_sha256.clone(),
            };
            let (fact, processing) = match outcome {
                Some(true) => (
                    Fact::Output {
                        key,
                        attempt_id: *attempt,
                    },
                    Processing::OutputObserved(*attempt),
                ),
                outcome => {
                    let reason = if outcome.is_some() {
                        HoldReason::NoOutput
                    } else {
                        HoldReason::RequestUncertain
                    };
                    (
                        Fact::Hold {
                            key,
                            attempt_id: Some(*attempt),
                            reason: reason.clone(),
                        },
                        Processing::Held(Some(*attempt), reason),
                    )
                }
            };
            runtime.poisoned = true;
            self.write_external_input_fact(&runtime.owner, fact).await?;
            found.processing = processing;
            let _ = runtime.changed.send(id.clone());
            runtime.poisoned = false;
        }
        Ok(())
    }

    pub(crate) async fn hold_abandoned_external_input_claims(&self) -> Result<()> {
        let claims = {
            let state = self.external_input.lock().await;
            let Some(runtime) = state.as_ref() else {
                return Ok(());
            };
            runtime
                .recovery
                .messages
                .iter()
                .filter_map(|(id, found)| {
                    if let Processing::Claimed(attempt) = found.processing {
                        Some((id.clone(), attempt))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        self.finish_external_input_attempt(&claims, /*outcome*/ None)
            .await
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "the interruption gate must stay locked through durable publication"
    )]
    /// Returns whether the gate changed.
    pub(crate) async fn external_input_gate(&self, paused: bool) -> Result<bool> {
        let mut state = self.external_input.lock().await;
        let Some(runtime) = state.as_mut() else {
            return Ok(false);
        };
        ensure!(!runtime.poisoned, "external input recovery required");
        if runtime.paused == paused {
            return Ok(false);
        }
        runtime.poisoned = true;
        // On failure or cancellation, both the gate and poison remain closed.
        runtime.paused = true;
        runtime.permits.clear();
        self.write_external_input_fact(
            &runtime.owner,
            Fact::DispatchGate {
                paused,
                reason: codex_protocol::external_input_record::GateReason::Interrupted,
            },
        )
        .await?;
        runtime.paused = paused;
        runtime.poisoned = false;
        Ok(true)
    }

    pub(super) async fn write_external_input_fact(&self, owner: &str, fact: Fact) -> Result<()> {
        let live = self
            .services
            .live_thread
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("external input writer unavailable"))?;
        let record = Record {
            version: 1,
            owner_id: owner.into(),
            thread_id: self.thread_id.to_string(),
            fact,
        };
        live.append_items(&[RolloutItem::ExternalInput(record)])
            .await?;
        live.flush().await?;
        Ok(())
    }
}
