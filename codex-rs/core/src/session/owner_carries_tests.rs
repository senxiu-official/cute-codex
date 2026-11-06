use super::*;
use crate::context::CanonicalBytePolicy;
use crate::external_input::Runtime;
use crate::session::tests::attach_thread_persistence;
use crate::session::tests::make_session_and_context;
use crate::state::TaskKind;
use crate::tasks::SessionTask;
use crate::tasks::SessionTaskResult;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Message;
use codex_protocol::external_input::Source;
use codex_protocol::external_input::SourceKind;
use codex_protocol::external_input::recover;
use codex_protocol::external_input_status::DeliveryState;
use codex_protocol::protocol::TurnAbortReason;
use codex_rollout::RolloutRecorder;
use codex_thread_store::PersistContext;
use pretty_assertions::assert_eq;
use tokio_util::sync::CancellationToken;

fn envelope(session: &Session, id: &str) -> Envelope {
    let mut value = Envelope {
        version: 1,
        owner_id: "owner".into(),
        thread_id: session.thread_id.to_string(),
        runtime_generation: 7,
        message: Message {
            id: id.into(),
            source: Source {
                kind: SourceKind::Service,
                id: "alarm-service".into(),
            },
            event_type: "alarm".into(),
            delivery: Delivery::Soon,
            text: "external notice; never owner authorization".into(),
        },
        semantic_sha256: String::new(),
        view: None,
    };
    value.semantic_sha256 = value.digest();
    value
}

async fn fixture() -> (Arc<Session>, Arc<TurnContext>, std::path::PathBuf) {
    let (mut session, turn) = make_session_and_context().await;
    let path = attach_thread_persistence(&mut session).await;
    // These tests run the owner boundary directly (WaitingTask never executes
    // RegularTask's lifecycle). Strict recovery must still see the real turn
    // containing the canonical pair, just as it does in a regular runtime turn.
    let live = session.services.live_thread.as_ref().unwrap();
    live.append_items(
        &[codex_history::RolloutItem::EventMsg(EventMsg::TurnStarted(
            codex_protocol::protocol::TurnStartedEvent {
                turn_id: turn.sub_id.clone(),
                turn_attribution: None,
                root_turn_id: None,
                trace_id: None,
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: ModeKind::Default,
            },
        ))],
    )
    .await
    .unwrap();
    live.flush().await.unwrap();
    live.persist_for_read().await.unwrap();
    *session.external_input.lock().await = Some(Runtime {
        policy: CanonicalBytePolicy::default(),
        policy_blocked: Default::default(),
        dispatch_revision: 1,
        attempted_revision: 0,
        retry_exclusions: Default::default(),
        changed: tokio::sync::broadcast::channel(100).0,
        owner: "owner".into(),
        recovery: recover("owner", &session.thread_id.to_string(), &[]).unwrap(),
        pending: Vec::new(),
        owner_carries: Default::default(),
        paused: false,
        poisoned: false,
        retries: Default::default(),
        permits: Default::default(),
        exclusive_turn: None,
    });
    (Arc::new(session), Arc::new(turn), path)
}

fn request(envelopes: Vec<Envelope>) -> TurnInputRequest {
    TurnInputRequest::user_input(vec![UserInput::Text {
        text: "plain owner text".into(),
        text_elements: Vec::new(),
    }])
    .with_owner_carries(envelopes)
}

#[derive(Clone)]
struct WaitingTask;
impl SessionTask for WaitingTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }
    fn span_name(&self) -> &'static str {
        "owner_carries_test"
    }
    async fn run(
        self: Arc<Self>,
        _session: Arc<Session>,
        _ctx: Arc<TurnContext>,
        _input: Vec<TurnInput>,
        cancellation_token: CancellationToken,
    ) -> SessionTaskResult {
        cancellation_token.cancelled().await;
        Ok(None)
    }
}

#[tokio::test]
async fn owner_carries_busy_start_and_steer_wait_for_the_specific_owner_boundary() {
    for steer_only in [false, true] {
        let (session, turn, path) = fixture().await;
        session
            .spawn_task(Arc::clone(&turn), Vec::new(), WaitingTask)
            .await;
        let value = envelope(&session, "busy-notice");
        let mode = if steer_only {
            TurnInputMode::Steer {
                expected_turn_id: turn.sub_id.clone(),
            }
        } else {
            TurnInputMode::StartOrSteer
        };
        let submission = handle(
            &session,
            request(vec![value.clone()]),
            mode,
            "owner-request".into(),
        )
        .await
        .unwrap();
        assert!(matches!(submission, TurnInputSubmission::Steered { .. }));
        // A running tool reaches its normal drain BEFORE owner input processing.
        // It cannot consume the carrier or generate its durable receipt.
        session
            .consume_external_input(&turn, crate::external_input::InputBoundary::Drain)
            .await
            .unwrap();
        let status = session
            .external_input_status("owner", &value.message.id, &value.semantic_sha256)
            .await
            .unwrap();
        assert_eq!(status.delivery_state, DeliveryState::Pending);
        assert!(status.receipt.is_none());
        assert!(
            session
                .external_input
                .lock()
                .await
                .as_ref()
                .unwrap()
                .pending
                .is_empty()
        );
        // Replay refuses the entire owner submission: no second owner input is queued.
        assert!(
            handle(
                &session,
                request(vec![value.clone()]),
                TurnInputMode::StartOrSteer,
                "replay".into()
            )
            .await
            .is_err()
        );
        let pending = session
            .input_queue
            .get_pending_input(&session.active_turn)
            .await
            .0;
        assert_eq!(pending.len(), 1);
        assert!(
            !super::super::turn::run_hooks_and_record_inputs(
                &session,
                &turn,
                &turn.capture_current_model_info(),
                &pending,
                PersistContext::SteeredUserInput
            )
            .await
        );
        assert_pair_and_plain_owner(&session, &turn, &path, &value, 1).await;
        assert!(
            handle(
                &session,
                request(vec![value]),
                TurnInputMode::StartOrSteer,
                "committed-replay".into()
            )
            .await
            .is_err()
        );
        assert!(
            session
                .input_queue
                .get_pending_input(&session.active_turn)
                .await
                .0
                .is_empty()
        );
        session.abort_all_tasks(TurnAbortReason::Interrupted).await;
    }
}

async fn assert_pair_and_plain_owner(
    session: &Session,
    turn: &TurnContext,
    path: &std::path::Path,
    envelope: &Envelope,
    expected_count: usize,
) {
    let status = session
        .external_input_status("owner", &envelope.message.id, &envelope.semantic_sha256)
        .await
        .unwrap();
    assert_eq!(status.delivery_state, DeliveryState::ContextPersisted);
    let receipt = status.receipt.unwrap();
    assert_eq!(receipt.turn_id, turn.sub_id);
    let (items, _, errors) = RolloutRecorder::load_rollout_items(path).await.unwrap();
    assert_eq!(errors, 0);
    let restored =
        crate::external_input::recovery::restore("owner", &session.thread_id.to_string(), &items)
            .unwrap();
    assert_eq!(restored.recovery.messages.len(), expected_count);
    // Never weaken restore to accept a receipt whose actual consuming turn
    // cannot be established from durable lifecycle facts.
    let without_turn_start = items
        .iter()
        .filter(|item| {
            !matches!(
                item,
                codex_history::RolloutItem::EventMsg(EventMsg::TurnStarted(_))
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        crate::external_input::recovery::restore(
            "owner",
            &session.thread_id.to_string(),
            &without_turn_start
        )
        .is_err()
    );
    assert_eq!(
        restored.recovery.messages[&envelope.message.id]
            .commit
            .receipt,
        receipt
    );
    let canonical = envelope.response_item().unwrap();
    assert_eq!(items.iter().filter(|item| matches!(item, codex_history::RolloutItem::ResponseItem(found) if found.item == canonical)).count(), 1);
    let owners = items
        .iter()
        .filter_map(|item| match item {
            codex_history::RolloutItem::ResponseItem(found) => match &found.item {
                ResponseItem::Message { role, content, .. } if role == "user" => Some(content),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    let owner_index = items.iter().position(|item| matches!(item, codex_history::RolloutItem::ResponseItem(found) if matches!(&found.item, ResponseItem::Message { role, .. } if role == "user"))).unwrap();
    let canonical_index = items.iter().position(|item| matches!(item, codex_history::RolloutItem::ResponseItem(found) if found.item == canonical)).unwrap();
    assert!(owner_index < canonical_index);
    assert_eq!(owners.len(), 1);
    assert_eq!(
        owners[0],
        &vec![codex_protocol::models::ContentItem::InputText {
            text: "plain owner text".into()
        }]
    );
}

#[tokio::test]
async fn owner_carries_initial_input_records_plain_owner_then_canonical_pair() {
    let (session, turn, path) = fixture().await;
    let value = envelope(&session, "initial-notice");
    let mut second = envelope(&session, "second-notice");
    second.message.source = Source {
        kind: SourceKind::Agent,
        id: "other-agent".into(),
    };
    second.message.event_type = "delegation_result".into();
    second.semantic_sha256 = second.digest();
    let carries = session
        .reserve_owner_carries(vec![value.clone(), second.clone()])
        .await
        .unwrap();
    let mut input = pending_turn_input(
        &session,
        request(Vec::new()).input,
        None,
        &turn.sub_id,
        UserInputOrigin::User,
    )
    .await;
    let TurnInput::UserInput { metadata, .. } = &mut input else {
        panic!("owner input")
    };
    metadata.owner_carries = Some(carries);
    assert!(
        !super::super::turn::run_hooks_and_record_inputs(
            &session,
            &turn,
            &turn.capture_current_model_info(),
            &[input],
            PersistContext::Standard
        )
        .await
    );
    assert_pair_and_plain_owner(&session, &turn, &path, &value, 2).await;
    assert_pair_and_plain_owner(&session, &turn, &path, &second, 2).await;
    let first = session
        .external_input_status("owner", &value.message.id, &value.semantic_sha256)
        .await
        .unwrap()
        .receipt
        .unwrap();
    let second = session
        .external_input_status("owner", &second.message.id, &second.semantic_sha256)
        .await
        .unwrap()
        .receipt
        .unwrap();
    assert!(first.ordinal < second.ordinal);
    assert_ne!(first.response_item_id, second.response_item_id);
}

#[tokio::test]
async fn owner_carries_refused_steer_is_unknown_and_can_be_retried() {
    let (session, turn, _path) = fixture().await;
    let value = envelope(&session, "refused");
    let submission = handle(
        &session,
        request(vec![value.clone()]),
        TurnInputMode::Steer {
            expected_turn_id: turn.sub_id.clone(),
        },
        "owner".into(),
    )
    .await
    .unwrap();
    assert_eq!(
        submission,
        TurnInputSubmission::NotSubmitted {
            reason: NotSubmittedReason::NoActiveTurn
        }
    );
    let status = session
        .external_input_status("owner", &value.message.id, &value.semantic_sha256)
        .await
        .unwrap();
    assert_eq!(status.delivery_state, DeliveryState::Unknown);
    assert!(status.receipt.is_none());
    let reservation = session
        .reserve_owner_carries(vec![value.clone()])
        .await
        .unwrap();
    drop(reservation);
    assert_eq!(
        session
            .external_input_status("owner", &value.message.id, &value.semantic_sha256)
            .await
            .unwrap()
            .delivery_state,
        DeliveryState::Unknown
    );
}

#[tokio::test]
async fn owner_carries_batch_validation_is_all_or_nothing_and_cancel_releases_identity() {
    let (session, _turn, _path) = fixture().await;
    let good = envelope(&session, "good");
    let mut bad = envelope(&session, "bad");
    bad.message.text.push_str("tampered digest");
    assert!(
        session
            .reserve_owner_carries(vec![good.clone(), bad])
            .await
            .is_err()
    );
    assert_eq!(
        session
            .external_input_status("owner", &good.message.id, &good.semantic_sha256)
            .await
            .unwrap()
            .delivery_state,
        DeliveryState::Unknown
    );
    assert!(
        session
            .reserve_owner_carries(vec![good.clone(), good.clone()])
            .await
            .is_err()
    );
    let reservation = session
        .reserve_owner_carries(vec![good.clone()])
        .await
        .unwrap();
    let mut conflict = good.clone();
    conflict.message.text.push('!');
    conflict.semantic_sha256 = conflict.digest();
    assert_eq!(
        session
            .external_input_status("owner", &good.message.id, &conflict.semantic_sha256)
            .await
            .unwrap()
            .delivery_state,
        DeliveryState::Conflict
    );
    assert!(session.reserve_owner_carries(vec![conflict]).await.is_err());
    drop(reservation);
    assert_eq!(
        session
            .external_input_status("owner", &good.message.id, &good.semantic_sha256)
            .await
            .unwrap()
            .delivery_state,
        DeliveryState::Unknown
    );
    let retry = session.reserve_owner_carries(vec![good]).await.unwrap();
    assert_eq!(retry.len(), 1);
}

#[tokio::test]
async fn owner_carries_expected_turn_refusal_leaves_no_queue_or_receipt() {
    let (session, turn, _path) = fixture().await;
    session
        .spawn_task(Arc::clone(&turn), Vec::new(), WaitingTask)
        .await;
    let value = envelope(&session, "wrong-turn");
    let submission = handle(
        &session,
        request(vec![value.clone()]),
        TurnInputMode::Steer {
            expected_turn_id: "wrong-turn".into(),
        },
        "refused-owner".into(),
    )
    .await
    .unwrap();
    assert!(matches!(
        submission,
        TurnInputSubmission::NotSubmitted {
            reason: NotSubmittedReason::ExpectedTurnMismatch { .. }
        }
    ));
    assert!(
        session
            .input_queue
            .get_pending_input(&session.active_turn)
            .await
            .0
            .is_empty()
    );
    let status = session
        .external_input_status("owner", &value.message.id, &value.semantic_sha256)
        .await
        .unwrap();
    assert_eq!(status.delivery_state, DeliveryState::Unknown);
    assert!(status.receipt.is_none());
    session.abort_all_tasks(TurnAbortReason::Interrupted).await;
}

#[tokio::test]
async fn owner_carries_uncertain_write_blocks_sampling_and_cannot_fabricate_receipt() {
    let (mut session, turn, _path) = fixture().await;
    Arc::get_mut(&mut session).unwrap().services.live_thread = None;
    let value = envelope(&session, "no-writer");
    let carries = session.reserve_owner_carries(vec![value]).await.unwrap();
    let mut input = pending_turn_input(
        &session,
        request(Vec::new()).input,
        None,
        &turn.sub_id,
        UserInputOrigin::User,
    )
    .await;
    let TurnInput::UserInput { metadata, .. } = &mut input else {
        panic!("owner input")
    };
    metadata.owner_carries = Some(carries);
    assert!(
        super::super::turn::run_hooks_and_record_inputs(
            &session,
            &turn,
            &turn.capture_current_model_info(),
            &[input],
            PersistContext::Standard
        )
        .await
    );
    assert!(
        session
            .ensure_external_input_policy_allows_sampling()
            .await
            .is_err()
    );
    let state = session.external_input.lock().await;
    let runtime = state.as_ref().unwrap();
    assert!(runtime.poisoned);
    assert!(runtime.recovery.messages.is_empty());
}

#[tokio::test]
async fn owner_carries_rejected_hook_releases_even_if_old_input_batch_is_retained() {
    let (session, _turn, _path) = fixture().await;
    let value = envelope(&session, "hook-rejected");
    let rejected = session
        .reserve_owner_carries(vec![value.clone()])
        .await
        .unwrap();
    session.discard_owner_carries(&rejected).await;
    let status = session
        .external_input_status("owner", &value.message.id, &value.semantic_sha256)
        .await
        .unwrap();
    assert_eq!(status.delivery_state, DeliveryState::Unknown);
    assert!(status.receipt.is_none());
    let retry = session
        .reserve_owner_carries(vec![value.clone()])
        .await
        .unwrap();
    // An old hook result may never discard a newer reservation with the same id.
    session.discard_owner_carries(&rejected).await;
    assert_eq!(
        session
            .external_input_status("owner", &value.message.id, &value.semantic_sha256)
            .await
            .unwrap()
            .delivery_state,
        DeliveryState::Pending
    );
    drop(retry);
}
