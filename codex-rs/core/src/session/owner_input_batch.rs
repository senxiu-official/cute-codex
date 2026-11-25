//! Owner batches use the idle reservation and a durable, model-invisible intent.
//! An intent is never an in-context receipt and is never replayed as input.
use super::*;
use codex_protocol::protocol::EventMsg;
use codex_history::RolloutItem;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::OwnerInputBatchAdmission;
type OwnerInput = codex_protocol::turn_input::OwnerInput<ResolvedUserInput>;
use codex_rollout::RolloutRecorder;
use std::collections::BTreeSet;

pub(super) fn validate(request: &TurnInputRequest, mode: &TurnInputMode) -> CodexResult<()> {
    let prefix = request.owner_inputs.as_ref().expect("batch present");
    let SubmittedTurnInput::UserInput {
        content,
        client_id: Some(id),
    } = &request.input
    else {
        return Err(CodexErr::InvalidRequest(
            "ownerInputs requires an identified owner carrier".into(),
        ));
    };
    if !matches!(
        mode,
        TurnInputMode::StartOrSteer | TurnInputMode::StartIfIdle
    ) || request.start_origin() != TurnStartOrigin::Explicit
        || !request.owner_carries.is_empty()
        || prefix.len() >= 100
    {
        return Err(CodexErr::InvalidRequest(
            "invalid ownerInputs admission mode or size".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for (id, content) in prefix
        .iter()
        .map(|item| (&item.client_user_message_id, &item.input))
        .chain(std::iter::once((id, content)))
    {
        if id.is_empty()
            || id.len() > 256
            || !ids.insert(id)
            || content.is_empty()
            || content.iter().any(|part| {
                matches!(
                    part.to_user_input(),
                    UserInput::Audio { .. } | UserInput::LocalAudio { .. }
                )
            })
        {
            return Err(CodexErr::InvalidRequest(
                "invalid or duplicate ownerInputs identity/input".into(),
            ));
        }
    }
    Ok(())
}

fn ids(prefix: &[OwnerInput], carrier: &SubmittedTurnInput) -> Vec<String> {
    let SubmittedTurnInput::UserInput {
        client_id: Some(id),
        ..
    } = carrier
    else {
        unreachable!()
    };
    prefix
        .iter()
        .map(|item| item.client_user_message_id.clone())
        .chain(std::iter::once(id.clone()))
        .collect()
}

pub(super) async fn check(
    session: &Session,
    prefix: &[OwnerInput],
    carrier: &SubmittedTurnInput,
) -> CodexResult<Option<NotSubmittedReason>> {
    if *session.owner_batch_uncertain.lock().await {
        return Ok(Some(NotSubmittedReason::OwnerInputBatchRecoveryRequired));
    }
    let live = session.services.live_thread.as_ref().ok_or_else(|| {
        CodexErr::InvalidRequest("ownerInputs requires persistent history".into())
    })?;
    let result = async {
        live.flush().await?;
        let path = live
            .local_rollout_path()
            .await?
            .ok_or_else(|| anyhow::anyhow!("ownerInputs requires local history"))?;
        let (items, thread_id, parse_errors) = RolloutRecorder::load_rollout_items(&path).await?;
        anyhow::ensure!(
            thread_id == Some(session.thread_id) && parse_errors == 0,
            "ownerInputs history is incomplete"
        );
        Ok::<_, anyhow::Error>(items)
    }
    .await;
    let result = match result {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!(%error, "ownerInputs history unavailable before admission");
            return Ok(Some(NotSubmittedReason::OwnerInputBatchHistoryUnavailable));
        }
    };
    let submitted: BTreeSet<_> = ids(prefix, carrier).into_iter().collect();
    if result.iter().any(|item| match item {
        RolloutItem::TurnContext(context) => context.owner_input_batch.as_ref().is_some_and(|batch|
            batch.thread_id == session.thread_id && batch.client_ids.iter().any(|id| submitted.contains(id))),
        RolloutItem::EventMsg(EventMsg::ItemCompleted(event)) if event.thread_id == session.thread_id => {
            matches!(&event.item, TurnItem::UserMessage(message) if message.client_id.as_ref().is_some_and(|id| submitted.contains(id)))
        }
        RolloutItem::EventMsg(EventMsg::UserMessage(message)) => message
            .client_id
            .as_ref()
            .is_some_and(|id| submitted.contains(id)),
        _ => false,
    }) {
        return Err(CodexErr::InvalidRequest("ownerInputs identity already admitted; query original IDs instead of replaying".into()));
    }
    Ok(None)
}

pub(super) async fn admit(
    session: &Session,
    context: &TurnContext,
    prefix: &[OwnerInput],
    carrier: &SubmittedTurnInput,
) -> CodexResult<()> {
    let live = session
        .services
        .live_thread
        .as_ref()
        .expect("checked persistent history");
    let mut record = context.to_turn_context_item();
    record.owner_input_batch = Some(OwnerInputBatchAdmission {
        thread_id: session.thread_id,
        client_ids: ids(prefix, carrier),
    });
    // Poison before the first write. Failure is unknown, never a safe-to-resend refusal.
    *session.owner_batch_uncertain.lock().await = true;
    let result = async {
        live.append_items(&[RolloutItem::TurnContext(record)])
            .await?;
        live.flush().await?;
        live.persist_for_read().await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    result.map_err(|error| {
        CodexErr::Fatal(format!(
            "ownerInputs outcome unknown; query original IDs, do not resend: {error}"
        ))
    })?;
    *session.owner_batch_uncertain.lock().await = false;
    Ok(())
}

pub(super) async fn finish_start(
    session: &Session,
    turn_state: &Arc<tokio::sync::Mutex<TurnState>>,
    submission: TurnInputSubmission,
) -> CodexResult<TurnInputSubmission> {
    if !matches!(submission, TurnInputSubmission::Started { .. }) {
        // Durable intent already seals these IDs if shutdown or interruption
        // wins task registration. A refusal here must never promise safe resend.
        *session.owner_batch_uncertain.lock().await = true;
        session.clear_reserved_idle_turn(turn_state).await;
        return Err(CodexErr::Fatal(
            "ownerInputs outcome unknown after durable admission; query original IDs, do not resend".into(),
        ));
    }
    Ok(submission)
}

#[cfg(test)]
mod tests {
    use super::*;
use codex_protocol::protocol::EventMsg;
    use crate::session::tests::{attach_thread_persistence, make_session_and_context};
    use codex_thread_store::PersistContext;

    fn text(value: &str) -> Vec<ResolvedUserInput> {
        vec![ResolvedUserInput::text(value.to_owned())]
    }

    fn batch() -> (Vec<OwnerInput>, SubmittedTurnInput) {
        (
            vec![
                OwnerInput {
                    client_user_message_id: "a".into(),
                    input: text("/literal/path"),
                },
                OwnerInput {
                    client_user_message_id: "b".into(),
                    input: text("second"),
                },
            ],
            SubmittedTurnInput::UserInput {
                content: text("draft"),
                client_id: Some("draft".into()),
            },
        )
    }

    #[tokio::test]
    async fn owner_input_batch_cancelled_reservation_cannot_recreate_a_turn() {
        let (session, context) = make_session_and_context().await;
        let session = Arc::new(session);
        let reserved = reserve_idle_turn(
            &session,
            IdleReservation::Origin(TurnStartOrigin::Explicit),
            None,
        )
        .await
        .unwrap();
        session.clear_reserved_idle_turn(&reserved).await;
        let outcome = session
            .start_task_with_reservation(
                Arc::new(context),
                Vec::new(),
                RegularTask::new(),
                Some(&reserved),
            )
            .await;
        assert!(matches!(
            outcome,
            TurnInputSubmission::NotSubmitted {
                reason: NotSubmittedReason::Interrupted,
            }
        ));
        assert!(session.active_turn.lock().await.is_none());
    }

    #[tokio::test]
    async fn owner_input_batch_cancelled_after_durable_intent_remains_unknown() {
        let (mut session, context) = make_session_and_context().await;
        let _path = attach_thread_persistence(&mut session).await;
        let session = Arc::new(session);
        let reserved = reserve_idle_turn(
            &session,
            IdleReservation::Origin(TurnStartOrigin::Explicit),
            None,
        )
        .await
        .unwrap();
        let (prefix, carrier) = batch();
        admit(&session, &context, &prefix, &carrier).await.unwrap();
        session.clear_reserved_idle_turn(&reserved).await;
        let submission = session
            .start_task_with_reservation(
                Arc::new(context),
                Vec::new(),
                RegularTask::new(),
                Some(&reserved),
            )
            .await;
        let error = finish_start(&session, &reserved, submission)
            .await
            .unwrap_err();
        assert!(matches!(
            error.details(),
            codex_protocol::error::CodexErrorDetails::Fatal(_)
        ));
        assert!(session.active_turn.lock().await.is_none());
        let fresh = SubmittedTurnInput::UserInput {
            content: text("fresh"),
            client_id: Some("never-admitted".into()),
        };
        assert_eq!(
            check(&session, &[], &fresh).await.unwrap(),
            Some(NotSubmittedReason::OwnerInputBatchRecoveryRequired)
        );
    }

    #[tokio::test]
    async fn owner_input_batch_durable_intent_blocks_full_partial_and_unknown_replay() {
        let (mut session, context) = make_session_and_context().await;
        let path = attach_thread_persistence(&mut session).await;
        let (prefix, carrier) = batch();
        check(&session, &prefix, &carrier).await.unwrap();
        admit(&session, &context, &prefix, &carrier).await.unwrap();
        // No owner item or model request exists yet: losing the RPC response must
        // still reserve every ID. Recovery reads the intent, not a volatile set.
        assert!(check(&session, &prefix, &carrier).await.is_err());
        assert!(check(&session, &[], &carrier).await.is_err());
        let changed_carrier = SubmittedTurnInput::UserInput {
            content: text("new"),
            client_id: Some("new".into()),
        };
        assert!(
            check(&session, &prefix[..1], &changed_carrier)
                .await
                .is_err()
        );
        let (items, _, errors) = RolloutRecorder::load_rollout_items(&path).await.unwrap();
        assert_eq!(errors, 0);
        assert_eq!(items.iter().filter(|item| matches!(item, RolloutItem::TurnContext(context) if context.owner_input_batch.is_some())).count(), 1);
        assert!(
            !items
                .iter()
                .any(|item| matches!(item, RolloutItem::EventMsg(EventMsg::ItemCompleted(_))))
        );
        *session.owner_batch_uncertain.lock().await = true;
        assert_eq!(
            check(&session, &[], &changed_carrier).await.unwrap(),
            Some(NotSubmittedReason::OwnerInputBatchRecoveryRequired)
        );
    }

    #[tokio::test]
    async fn owner_input_batch_records_each_original_id_in_order() {
        let (mut session, context) = make_session_and_context().await;
        let path = attach_thread_persistence(&mut session).await;
        let session = Arc::new(session);
        let context = Arc::new(context);
        let (prefix, carrier) = batch();
        check(&session, &prefix, &carrier).await.unwrap();
        let metadata = UserInputMetadata {
            owner_input_batch: true,
            ..Default::default()
        };
        let mut input = Vec::new();
        for owner in prefix {
            input.push(
                pending_turn_input(
                    &session,
                    SubmittedTurnInput::UserInput {
                        content: owner.input,
                        client_id: Some(owner.client_user_message_id),
                    },
                    None,
                    &context.sub_id,
                    metadata.clone(),
                )
                .await,
            );
        }
        input.push(pending_turn_input(&session, carrier, None, &context.sub_id, metadata).await);
        assert!(
            !crate::session::turn::run_hooks_and_record_inputs(
                &session,
                &context,
                &context.capture_current_model_info(),
                &input,
                PersistContext::Standard
            )
            .await
            .unwrap()
        );
        let live = session.services.live_thread.as_ref().unwrap();
        live.flush().await.unwrap();
        live.persist_for_read().await.unwrap();
        let (items, _, errors) = RolloutRecorder::load_rollout_items(&path).await.unwrap();
        assert_eq!(errors, 0);
        let ids: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                RolloutItem::EventMsg(EventMsg::ItemCompleted(event)) => match &event.item {
                    TurnItem::UserMessage(message) => message.client_id.as_deref(),
                    _ => None,
                },
                RolloutItem::EventMsg(EventMsg::UserMessage(message)) => {
                    message.client_id.as_deref()
                }
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec!["a", "b", "draft"]);
        // A historical singleton has no batch intent. Its retained original ID
        // must also prevent partial batch admission in legacy history mode.
        let (prefix, carrier) = batch();
        assert!(check(&session, &prefix, &carrier).await.is_err());
    }

    #[tokio::test]
    async fn owner_input_batch_failed_admission_poison_is_not_new_batch_unknown() {
        let (mut session, context) = make_session_and_context().await;
        attach_thread_persistence(&mut session).await;
        let (prefix, carrier) = batch();
        assert_eq!(check(&session, &prefix, &carrier).await.unwrap(), None);
        // Close the real private persistence writer between preflight and admit.
        session
            .services
            .live_thread
            .as_ref()
            .unwrap()
            .shutdown()
            .await
            .unwrap();
        let error = admit(&session, &context, &prefix, &carrier)
            .await
            .unwrap_err();
        assert!(matches!(
            error.details(),
            codex_protocol::error::CodexErrorDetails::Fatal(_)
        ));
        assert!(*session.owner_batch_uncertain.lock().await);
        let new_carrier = SubmittedTurnInput::UserInput {
            content: text("fresh"),
            client_id: Some("fresh".into()),
        };
        assert_eq!(
            check(&session, &[], &new_carrier).await.unwrap(),
            Some(NotSubmittedReason::OwnerInputBatchRecoveryRequired)
        );
    }

    #[tokio::test]
    async fn owner_input_batch_unreadable_history_is_not_admitted() {
        let (mut session, _) = make_session_and_context().await;
        let path = attach_thread_persistence(&mut session).await;
        let live = session.services.live_thread.as_ref().unwrap();
        live.flush().await.unwrap();
        live.persist_for_read().await.unwrap();
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .await
            .unwrap();
        file.write_all(b"\n{invalid-private-history\n")
            .await
            .unwrap();
        file.flush().await.unwrap();
        let (prefix, carrier) = batch();
        assert_eq!(
            check(&session, &prefix, &carrier).await.unwrap(),
            Some(NotSubmittedReason::OwnerInputBatchHistoryUnavailable)
        );
        assert!(!*session.owner_batch_uncertain.lock().await);
    }

    #[tokio::test]
    async fn owner_input_batch_yields_to_trigger_mailbox_without_admission() {
        let (mut session, _) = make_session_and_context().await;
        attach_thread_persistence(&mut session).await;
        let session = Arc::new(session);
        session
            .input_queue
            .enqueue_mailbox_communication(
                codex_protocol::protocol::InterAgentCommunication::new(
                    codex_protocol::AgentPath::root(),
                    codex_protocol::AgentPath::root(),
                    Vec::new(),
                    "pending trigger".into(),
                    true,
                )
                .into(),
                Default::default(),
            )
            .await;
        let (prefix, carrier) = batch();
        let admission = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let outcome = handle(
            &session,
            TurnInputRequest::new(carrier.clone())
                .with_owner_inputs(Some(prefix.clone()))
                .with_owner_input_batch_admission(Arc::clone(&admission)),
            TurnInputMode::StartOrSteer,
            "batch-behind-mailbox".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            TurnInputSubmission::NotSubmitted {
                reason: NotSubmittedReason::PendingTriggerTurn,
            }
        );
        assert!(session.active_turn.lock().await.is_none());
        assert!(session.input_queue.has_trigger_turn_mailbox_items().await);
        assert!(!admission.load(std::sync::atomic::Ordering::Acquire));
        assert!(!*session.owner_batch_uncertain.lock().await);
        // No member ID was sealed; the same full batch remains eligible later.
        assert_eq!(check(&session, &prefix, &carrier).await.unwrap(), None);
    }

    #[tokio::test]
    async fn owner_input_batch_refuses_active_turn_without_steering() {
        let (session, _) = make_session_and_context().await;
        let session = Arc::new(session);
        *session.active_turn.lock().await = Some(ActiveTurn::default());
        let (prefix, carrier) = batch();
        let admission = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let outcome = handle(
            &session,
            TurnInputRequest::new(carrier)
                .with_owner_inputs(Some(prefix))
                .with_owner_input_batch_admission(Arc::clone(&admission)),
            TurnInputMode::StartOrSteer,
            "batch".into(),
        )
        .await
        .unwrap();
        assert!(matches!(
            outcome,
            TurnInputSubmission::NotSubmitted {
                reason: NotSubmittedReason::NotIdle
            }
        ));
        assert!(!*session.owner_batch_uncertain.lock().await);
        assert!(!admission.load(std::sync::atomic::Ordering::Acquire));
    }
}
