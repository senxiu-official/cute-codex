use super::session::Session;
use super::turn_context::TurnContext;
use crate::context::ExternalInputContext;
use crate::external_input::InputBoundary;
use crate::external_input::Pending;
use crate::external_input::Runtime;
use crate::external_input::recovery::restore;
use crate::stream_events_utils::mark_thread_memory_mode_polluted_if_external_context;
use anyhow::Result;
use anyhow::ensure;
use codex_history::RolloutItem;
use codex_protocol::external_input::Commit;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::HoldReason;
use codex_protocol::external_input::Processing;
use codex_protocol::external_input::Receipt;
use codex_protocol::external_input::Recovered;
use codex_protocol::external_input_record::Fact;
use codex_protocol::external_input_record::Record;
use codex_protocol::external_input_status::DeliveryState;
use codex_protocol::external_input_status::Status;
use codex_protocol::protocol::TruncationPolicy;
use codex_rollout::RolloutRecorder;
use std::collections::BTreeMap;
use std::sync::Arc;

impl Session {
    /// Also guards compaction, which can sample before the regular input boundary.
    pub(crate) async fn ensure_external_input_policy_allows_sampling(&self) -> Result<()> {
        let state = self.external_input.lock().await;
        if let Some(runtime) = state.as_ref() {
            ensure!(
                runtime.policy_blocked.is_empty(),
                "receiver canonical byte policy blocks restored history; adjust trusted launch policy and restart"
            );
        }
        Ok(())
    }

    /// Canonical ExternalInput items that compaction must keep verbatim.
    ///
    /// These are the inputs of the compacting turn (committed with its turn id: the input that
    /// started it and any delivered while it runs) plus obligations that are still pending or
    /// claimed. Without them the model's latest visible input after compaction would be an older
    /// user message. Earlier, already processed events may be summarized away like tool output.
    /// The items are byte-identical to the ledger's canonical items, so claim validation and the
    /// boundary's missing-item restore keep recognizing them.
    pub(crate) async fn external_input_items_retained_by_compaction(
        &self,
        turn_id: &str,
    ) -> Vec<codex_protocol::models::ResponseItem> {
        let state = self.external_input.lock().await;
        let Some(runtime) = state.as_ref() else {
            return Vec::new();
        };
        let mut retained = runtime
            .recovery
            .messages
            .values()
            .filter(|found| {
                found.commit.receipt.turn_id == turn_id
                    || matches!(
                        found.processing,
                        Processing::Pending(_) | Processing::Claimed(_)
                    )
            })
            .collect::<Vec<_>>();
        retained.sort_unstable_by_key(|found| found.commit.receipt.ordinal);
        retained
            .into_iter()
            .filter_map(|found| found.commit.envelope.response_item().ok())
            .collect()
    }

    /// Called by the trusted launch owner before exposing the loaded thread.
    /// This never creates or resumes a thread or infers an owner from input text.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "publication must wait for complete recovery while admission remains excluded"
    )]
    pub(crate) async fn bind_external_input(
        &self,
        owner: &str,
        policy: crate::context::CanonicalBytePolicy,
    ) -> Result<()> {
        let mut state = self.external_input.lock().await;
        if let Some(runtime) = state.as_ref() {
            ensure!(
                runtime.owner == owner && !runtime.poisoned,
                "external input unavailable"
            );
            return Ok(());
        }
        let live = self
            .services
            .live_thread
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("external input requires persistent history"))?;
        let path = live
            .local_rollout_path()
            .await?
            .ok_or_else(|| anyhow::anyhow!("external input requires local history"))?;
        let (items, thread_id, parse_errors) = RolloutRecorder::load_rollout_items(&path).await?;
        ensure!(
            parse_errors == 0 && thread_id == Some(self.thread_id),
            "unreadable external input history"
        );
        let mut restored = restore(owner, &self.thread_id.to_string(), &items)?;
        let retries = items
            .iter()
            .filter_map(|item| match item {
                RolloutItem::ExternalInput(record) => match &record.fact {
                    Fact::Retry { retry_id, .. } => Some((retry_id.clone(), record.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        // Current policy is not history validity or receipt identity. Keep
        // historical receipts readable, but stop sampling until trusted restart
        // selects a policy that permits all recorded canonical external items.
        let mut policy_blocked = std::collections::BTreeSet::new();
        let mut history = self.state.lock().await;
        // Retained items stay in place; append only missing canonical items in
        // receipt order. This does not recreate their pre-compaction positions.
        let mut ordered = restored.recovery.messages.iter_mut().collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|(_, found)| found.commit.receipt.ordinal);
        for (id, found) in ordered {
            let item = match ExternalInputContext::new(&found.commit.envelope, policy) {
                Ok(context) => context.into_response_item(),
                Err(codex_protocol::external_input::Error::Invalid(
                    "receiver canonical byte policy exceeded",
                )) => {
                    policy_blocked.insert(id.clone());
                    if let Processing::Pending(attempt) = found.processing {
                        self.write_external_input_fact(
                            owner,
                            Fact::Hold {
                                key: codex_protocol::external_input_record::MessageKey {
                                    message_id: id.clone(),
                                    semantic_sha256: found.commit.envelope.semantic_sha256.clone(),
                                },
                                attempt_id: attempt,
                                reason: HoldReason::CanonicalSizePolicy,
                            },
                        )
                        .await?;
                        found.processing =
                            Processing::Held(attempt, HoldReason::CanonicalSizePolicy);
                    }
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if matches!(found.processing, Processing::Pending(_))
                && !history.history.raw_items().any(|old| old == &item)
            {
                history.history.record_items(
                    std::iter::once(&item),
                    TruncationPolicy::Bytes(serde_json::to_vec(&item)?.len()),
                );
            }
        }
        drop(history);
        *state = Some(Runtime {
            policy,
            policy_blocked,
            dispatch_revision: 1,
            attempted_revision: 0,
            retry_exclusions: BTreeMap::new(),
            changed: tokio::sync::broadcast::channel(100).0,
            owner: owner.to_owned(),
            recovery: restored.recovery,
            pending: Vec::new(),
            paused: restored.paused,
            poisoned: false,
            retries,
            permits: restored.permits,
            exclusive_turn: None,
        });
        Ok(())
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "status must observe one recovered state and its effective mode"
    )]
    pub(crate) async fn external_input_status(
        &self,
        owner: &str,
        id: &str,
        digest: &str,
    ) -> Result<Status> {
        let state = self.external_input.lock().await;
        let runtime = state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("external input disabled"))?;
        ensure!(
            runtime.owner == owner && !runtime.poisoned,
            "external input recovery required"
        );
        let mut status = snapshot(runtime, id, digest);
        if status.processing.state
            == codex_protocol::external_input_status::ProcessingState::Pending
            && self
                .state
                .lock()
                .await
                .session_configuration
                .step_settings
                .collaboration_mode
                .mode
                == codex_protocol::config_types::ModeKind::Plan
        {
            status.processing =
                (&Processing::Held(status.processing.attempt_id, HoldReason::PlanMode)).into();
        }
        Ok(status)
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "duplicate validation, receiver policy and admission must remain atomic"
    )]
    pub(crate) async fn admit_external_input(&self, envelope: Envelope) -> Result<(Status, bool)> {
        envelope.validate()?;
        ensure!(
            envelope.thread_id == self.thread_id.to_string(),
            "external input thread mismatch"
        );
        let mut state = self.external_input.lock().await;
        let runtime = state
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("external input disabled"))?;
        ensure!(
            runtime.owner == envelope.owner_id && !runtime.poisoned,
            "external input recovery required"
        );
        let found = snapshot(runtime, &envelope.message.id, &envelope.semantic_sha256);
        if found.delivery_state != DeliveryState::Unknown {
            return Ok((found, false));
        }
        ensure!(
            runtime.policy_blocked.is_empty(),
            "receiver canonical byte policy blocks restored history; adjust trusted launch policy and restart"
        );
        ExternalInputContext::new(&envelope, runtime.policy)?;
        ensure!(runtime.admission_load() < 100, "external input overloaded");
        let id = envelope.message.id.clone();
        let digest = envelope.semantic_sha256.clone();
        runtime.dispatch_revision = runtime
            .dispatch_revision
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("external input dispatch revision exhausted"))?;
        // Linearize admission with the active/reserved-turn snapshot. The
        // runtime lock keeps concurrent admissions in this same order.
        let active = self.active_turn.lock().await;
        let excluded_turn = active.as_ref().map(|turn| Arc::clone(&turn.turn_state));
        let running_turn = active
            .as_ref()
            .filter(|turn| turn.task.is_some())
            .map(|turn| Arc::clone(&turn.turn_state));
        runtime.pending.push(Pending {
            envelope,
            running_turn,
            excluded_turn,
            exclusive_turn: None,
        });
        drop(active);
        Ok((snapshot(runtime, &id, &digest), true))
    }

    /// Only a regular turn's safe input boundary calls this method. The durable
    /// pair precedes publication; ordinary best-effort record helpers are not used.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "pair flush and context publication must serialize against admission, status and interruption"
    )]
    pub(crate) async fn consume_external_input(
        &self,
        turn: &TurnContext,
        boundary: InputBoundary,
    ) -> Result<()> {
        let current_turn = self
            .active_turn
            .lock()
            .await
            .as_ref()
            .map(|turn| Arc::clone(&turn.turn_state));
        let mut state = self.external_input.lock().await;
        let Some(runtime) = state.as_mut() else {
            return Ok(());
        };
        ensure!(
            !runtime.poisoned,
            "external input recovery required before sampling"
        );
        ensure!(
            runtime.policy_blocked.is_empty(),
            "receiver canonical byte policy blocks restored history; adjust trusted launch policy and restart"
        );
        let soon_boundary = self.external_input_soon_boundary(turn, boundary).await;
        let live = self
            .services
            .live_thread
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("external input writer unavailable"))?;
        // A compaction summary is not the canonical input. Restore only released
        // pending obligations; held uncertain/no-output items require explicit retry.
        // Use the same missing-item placement rule as bind/resume.
        let mut ordered = runtime.recovery.messages.iter().collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|(_, found)| found.commit.receipt.ordinal);
        for (id, found) in ordered {
            if !matches!(found.processing, Processing::Pending(_))
                || (runtime.paused && !runtime.permits.contains(id))
                || runtime
                    .retry_exclusions
                    .get(id)
                    .zip(current_turn.as_ref())
                    .is_some_and(|(excluded, current)| Arc::ptr_eq(excluded, current))
            {
                continue;
            }
            let item = ExternalInputContext::new(&found.commit.envelope, runtime.policy)?
                .into_response_item();
            let mut history = self.state.lock().await;
            if !history.history.raw_items().any(|old| old == &item) {
                history.history.record_items(
                    std::iter::once(&item),
                    TruncationPolicy::Bytes(serde_json::to_vec(&item)?.len()),
                );
            }
        }
        let mut index = 0;
        // A retry permit belongs to an existing A4 item above. It never
        // authorizes new admission-to-context, including passive messages.
        while !runtime.paused && index < runtime.pending.len() {
            let pending = &runtime.pending[index];
            if runtime.excluded_by_exclusive_turn(pending, current_turn.as_ref()) {
                index += 1;
                continue;
            }
            // A bound item is due at its own turn's first boundary.
            if pending.exclusive_turn.is_none()
                && pending.envelope.message.delivery == Delivery::AfterTurn
                && pending
                    .excluded_turn
                    .as_ref()
                    .zip(current_turn.as_ref())
                    .is_some_and(|(excluded, current)| Arc::ptr_eq(excluded, current))
            {
                index += 1;
                continue;
            }
            if pending.exclusive_turn.is_none()
                && pending.envelope.message.delivery == Delivery::Soon
                && !soon_boundary
                    .as_ref()
                    .is_some_and(|boundary| runtime.can_consume_soon(pending, boundary))
            {
                index += 1;
                continue;
            }
            let envelope = pending.envelope.clone();
            let item = ExternalInputContext::new(&envelope, runtime.policy)?.into_response_item();
            let next_ordinal = runtime
                .recovery
                .next_ordinal
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("external input receipt ordinal exhausted"))?;
            let receipt = Receipt::new(
                &envelope,
                turn.sub_id.clone(),
                runtime.recovery.next_ordinal,
            )?;
            let view = envelope.view.clone();
            let commit = Commit { envelope, receipt };
            let record = Record {
                version: 1,
                owner_id: runtime.owner.clone(),
                thread_id: self.thread_id.to_string(),
                fact: Fact::Commit {
                    commit: Box::new(commit.clone()),
                },
            };
            runtime.poisoned = true;
            #[cfg(all(debug_assertions, unix))]
            crate::external_input::probe::barrier(live, "before_pair", &commit.envelope.message.id)
                .await?;
            live.append_items(&[
                RolloutItem::ExternalInput(record),
                RolloutItem::ResponseItem(item.clone().into()),
            ])
            .await?;
            live.flush().await?;
            live.persist_for_read().await?;
            #[cfg(all(debug_assertions, unix))]
            crate::external_input::probe::barrier(live, "after_flush", &commit.envelope.message.id)
                .await?;
            // Preserve upstream external-context memory policy without replacing
            // the authoritative pair barrier with a best-effort history write.
            mark_thread_memory_mode_polluted_if_external_context(self, turn, &item).await;
            self.state.lock().await.history.record_items(
                std::iter::once(&item),
                TruncationPolicy::Bytes(serde_json::to_vec(&item)?.len()),
            );
            runtime.recovery.next_ordinal = next_ordinal;
            let processing = if commit.envelope.message.delivery.is_active() {
                Processing::Pending(None)
            } else {
                Processing::None
            };
            runtime.recovery.messages.insert(
                commit.envelope.message.id.clone(),
                Recovered { commit, processing },
            );
            let _ = runtime
                .changed
                .send(runtime.pending[index].envelope.message.id.clone());
            runtime.pending.remove(index);
            runtime.poisoned = false;
            // The canonical pair already owns persistence. Publish the existing UI item only
            // after its original A4 barrier, without appending a second durable record.
            if let codex_protocol::models::ResponseItem::FunctionCallOutput {
                id: Some(id),
                name: Some(name),
                namespace,
                output,
                ..
            } = &item
            {
                self.send_event_raw_with_persistence(
                    codex_protocol::protocol::Event {
                        id: turn.sub_id.clone(),
                        msg: codex_protocol::protocol::EventMsg::ItemCompleted(
                            codex_protocol::protocol::ItemCompletedEvent {
                                thread_id: self.thread_id,
                                turn_id: turn.sub_id.clone(),
                                item: codex_protocol::items::TurnItem::FunctionCallOutput(
                                    codex_protocol::items::FunctionCallOutputItem {
                                        id: id.to_string(),
                                        name: name.clone(),
                                        namespace: namespace.clone(),
                                        output: output.body.clone(),
                                        external_input_view: view,
                                    },
                                ),
                                started_at_ms: None,
                                completed_at_ms: chrono::Utc::now().timestamp_millis(),
                            },
                        ),
                    },
                    /*persist*/ false,
                )
                .await;
            }
            self.send_raw_response_items(turn, std::slice::from_ref(&item))
                .await;
        }
        Ok(())
    }
}

pub(super) fn snapshot(runtime: &Runtime, id: &str, digest: &str) -> Status {
    let mut status = Status {
        message_id: id.into(),
        semantic_sha256: digest.into(),
        delivery_state: DeliveryState::Unknown,
        receipt: None,
        processing: (&Processing::None).into(),
    };
    if let Some(found) = runtime.recovery.messages.get(id) {
        if found.commit.envelope.semantic_sha256 != digest {
            status.delivery_state = DeliveryState::Conflict;
            return status;
        }
        status.delivery_state = DeliveryState::ContextPersisted;
        status.receipt = Some(found.commit.receipt.clone());
        status.processing = if runtime.policy_blocked.contains(id)
            && matches!(found.processing, Processing::Pending(_) | Processing::None)
        {
            let attempt = if let Processing::Pending(attempt) = found.processing {
                attempt
            } else {
                None
            };
            (&Processing::Held(attempt, HoldReason::CanonicalSizePolicy)).into()
        } else if runtime.paused
            && !runtime.permits.contains(id)
            && let Processing::Pending(attempt) = found.processing
        {
            (&Processing::Held(attempt, HoldReason::Interrupted)).into()
        } else {
            (&found.processing).into()
        };
    } else if let Some(found) = runtime
        .pending
        .iter()
        .find(|found| found.envelope.message.id == id)
    {
        status.delivery_state = if found.envelope.semantic_sha256 == digest {
            DeliveryState::Pending
        } else {
            DeliveryState::Conflict
        };
        if found.envelope.message.delivery.is_active() {
            status.processing = if runtime.paused {
                (&Processing::Held(None, HoldReason::Interrupted)).into()
            } else {
                (&Processing::Pending(None)).into()
            };
        }
    }
    status
}
