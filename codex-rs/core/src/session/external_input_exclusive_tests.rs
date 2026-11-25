use super::*;
use crate::external_input::InputBoundary;
use crate::external_input::Runtime;
use crate::external_input::SoonBoundary;
use codex_protocol::external_input::Commit;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Message;
use codex_protocol::external_input::Receipt;
use codex_protocol::external_input::Recovered;
use codex_protocol::external_input::Source;
use codex_protocol::external_input::SourceKind;
use codex_protocol::external_input::recover;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

fn runtime() -> Runtime {
    Runtime {
        policy: crate::context::CanonicalBytePolicy::default(),
        policy_blocked: Default::default(),
        dispatch_revision: 1,
        attempted_revision: 0,
        retry_exclusions: BTreeMap::new(),
        changed: tokio::sync::broadcast::channel(100).0,
        owner: "owner".into(),
        recovery: recover("owner", "thread", &[]).unwrap(),
        pending: Vec::new(),
        owner_carries: BTreeMap::new(),
        paused: false,
        stopped: false,
        poisoned: false,
        retries: BTreeMap::new(),
        permits: Default::default(),
        exclusive_turn: None,
    }
}

fn envelope(thread: &str, id: &str, delivery: Delivery) -> Envelope {
    let mut envelope = Envelope {
        view: None,
        version: 1,
        owner_id: "owner".into(),
        thread_id: thread.into(),
        runtime_generation: 7,
        message: Message {
            id: id.into(),
            source: Source {
                kind: SourceKind::Agent,
                id: "requester".into(),
            },
            event_type: "ASK".into(),
            delivery,
            text: format!("text of {id}"),
        },
        semantic_sha256: String::new(),
    };
    envelope.semantic_sha256 = envelope.digest();
    envelope
}

fn pending(id: &str, delivery: Delivery, bound: Option<&ExclusiveTurn>) -> Pending {
    Pending {
        envelope: envelope("thread", id, delivery),
        running_turn: None,
        excluded_turn: None,
        exclusive_turn: bound.cloned(),
    }
}

fn turn(id: &str) -> ExclusiveTurn {
    ExclusiveTurn {
        id: id.into(),
        state: Arc::new(Mutex::new(TurnState::default())),
        stop_answer: None,
    }
}

#[test]
fn exclusive_turn_takes_only_its_item_and_passive_input() {
    let exclusive = turn("exclusive");
    let other = turn("other");
    let mut runtime = runtime();
    runtime.exclusive_turn = Some(exclusive.clone());
    let bound = pending("bound", Delivery::AfterTurn, Some(&exclusive));
    let after_turn = pending("after-turn", Delivery::AfterTurn, None);
    let soon = pending("soon", Delivery::Soon, None);
    let passive = pending("passive", Delivery::Passive, None);

    let excluded = |current: &ExclusiveTurn| {
        [&bound, &after_turn, &soon, &passive]
            .map(|item| runtime.excluded_by_exclusive_turn(item, Some(&current.state)))
    };
    assert_eq!(excluded(&exclusive), [false, true, true, false]);
    // Outside the exclusive turn only the bound item is held back.
    assert_eq!(excluded(&other), [true, false, false, false]);

    let boundary = |current: &ExclusiveTurn| SoonBoundary {
        current: Arc::clone(&current.state),
        input: InputBoundary::Drain,
        accepts_mail: true,
    };
    assert!(!runtime.can_consume_soon(&soon, &boundary(&exclusive)));
    assert!(runtime.can_consume_soon(&soon, &boundary(&other)));
    let bound_soon = pending("bound-soon", Delivery::Soon, Some(&exclusive));
    assert!(!runtime.can_consume_soon(&bound_soon, &boundary(&exclusive)));

    // A soon item admitted during the exclusive turn does not extend it.
    runtime.pending.push(soon);
    assert!(!runtime.has_soon_continuation(&boundary(&exclusive)));
    assert!(runtime.has_soon_continuation(&boundary(&other)));
}

#[test]
fn bound_items_never_request_native_dispatch() {
    let exclusive = turn("exclusive");
    let mut runtime = runtime();
    runtime.exclusive_turn = Some(exclusive.clone());
    runtime
        .pending
        .push(pending("bound", Delivery::AfterTurn, Some(&exclusive)));
    assert!(!runtime.has_dispatch_work());
    runtime
        .pending
        .push(pending("other", Delivery::AfterTurn, None));
    assert!(runtime.has_dispatch_work());
}

async fn session_with_runtime(runtime: Runtime) -> Arc<Session> {
    let (session, _turn, _events) = crate::session::tests::make_session_and_context_with_rx().await;
    *session.external_input.lock().await = Some(runtime);
    session
}

async fn refusal(session: &Arc<Session>, id: &str) -> BusyReason {
    let value = envelope(&session.thread_id.to_string(), id, Delivery::AfterTurn);
    let refused = session
        .submit_exclusive_external_input(value.clone(), false)
        .await
        .expect("admission decision")
        .expect_err("exclusive admission refuses");
    // Refusal never queues: the item stays unknown and nothing is pending.
    let status = session
        .external_input_status("owner", id, &value.semantic_sha256)
        .await
        .unwrap();
    assert_eq!(status.delivery_state, DeliveryState::Unknown);
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
    refused
}

#[tokio::test]
#[expect(
    clippy::await_holding_invalid_type,
    reason = "the test edits recovery state under the admission lock"
)]
async fn exclusive_submit_refuses_busy_without_queueing() {
    let session = session_with_runtime(runtime()).await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());
    assert_eq!(refusal(&session, "busy").await, BusyReason::ActiveTurn);
    *session.active_turn.lock().await = None;

    session.external_input.lock().await.as_mut().unwrap().paused = true;
    assert_eq!(refusal(&session, "gate").await, BusyReason::GateClosed);
    session.external_input.lock().await.as_mut().unwrap().paused = false;

    let committed = envelope(&session.thread_id.to_string(), "committed", Delivery::Soon);
    let mut state = session.external_input.lock().await;
    let runtime = state.as_mut().unwrap();
    runtime.recovery = recover("owner", &session.thread_id.to_string(), &[]).unwrap();
    runtime.recovery.messages.insert(
        "committed".into(),
        Recovered {
            commit: Commit {
                receipt: Receipt::new(&committed, "old-turn".into(), 1).unwrap(),
                envelope: committed,
            },
            processing: Processing::Pending(None),
        },
    );
    drop(state);
    assert_eq!(
        refusal(&session, "obligation").await,
        BusyReason::PendingObligation
    );
    assert!(session.active_turn.lock().await.is_none());
}

#[tokio::test]
async fn exclusive_submit_requires_active_delivery() {
    let session = session_with_runtime(runtime()).await;
    let value = envelope(&session.thread_id.to_string(), "passive", Delivery::Passive);
    assert!(
        session
            .submit_exclusive_external_input(value, false)
            .await
            .is_err()
    );
    assert!(session.active_turn.lock().await.is_none());
}

#[tokio::test]
async fn release_drops_an_unconsumed_bound_item() {
    let session = session_with_runtime(runtime()).await;
    let exclusive = turn("exclusive");
    let mut changed = {
        let mut state = session.external_input.lock().await;
        let runtime = state.as_mut().unwrap();
        runtime.exclusive_turn = Some(exclusive.clone());
        runtime
            .pending
            .push(pending("bound", Delivery::AfterTurn, Some(&exclusive)));
        runtime
            .pending
            .push(pending("other", Delivery::AfterTurn, None));
        runtime.changed.subscribe()
    };
    session
        .release_exclusive_external_input(&turn("unrelated").state)
        .await;
    assert_eq!(
        session
            .external_input
            .lock()
            .await
            .as_ref()
            .unwrap()
            .pending
            .len(),
        2
    );
    session
        .release_exclusive_external_input(&exclusive.state)
        .await;
    let state = session.external_input.lock().await;
    let runtime = state.as_ref().unwrap();
    assert!(runtime.exclusive_turn.is_none());
    assert_eq!(
        runtime
            .pending
            .iter()
            .map(|pending| pending.envelope.message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["other"]
    );
    assert_eq!(changed.try_recv().unwrap(), "bound");
}

fn answer(thread: &str, id: &str) -> Envelope {
    let mut value = envelope(thread, id, Delivery::AfterTurn);
    value.message.source = Source {
        kind: SourceKind::Service,
        id: "cutex-asks".into(),
    };
    value.message.event_type = "ANSWER".into();
    value.semantic_sha256 = value.digest();
    value
}

#[tokio::test]
async fn stop_answer_reservation_is_scoped_to_one_turn_and_identity() {
    let mut runtime = runtime();
    runtime.paused = true;
    runtime.stopped = true;
    let session = session_with_runtime(runtime).await;
    let value = answer(&session.thread_id.to_string(), "answer");
    let reserved = session
        .reserve_exclusive_external_input(value.clone(), "answer-turn", true)
        .await
        .unwrap();
    let Reservation::Reserved(_, state) = reserved else {
        panic!("Stop ANSWER must reserve")
    };
    let mut external = session.external_input.lock().await;
    let runtime = external.as_mut().unwrap();
    assert!(runtime.paused);
    assert!(runtime.stopped);
    assert_eq!(
        snapshot(runtime, &value.message.id, &value.semantic_sha256)
            .processing
            .state,
        codex_protocol::external_input_status::ProcessingState::Pending
    );
    assert!(runtime.processing_allowed("answer", Some(&state)));
    assert!(!runtime.processing_allowed("other", Some(&state)));
    assert!(!runtime.processing_allowed("answer", Some(&turn("other").state)));
    assert!(!runtime.has_dispatch_work());
    let passive = pending("passive", Delivery::Passive, None);
    assert!(runtime.excluded_by_exclusive_turn(&passive, Some(&state)));
    // Even a retry permit for another message cannot extend this turn.
    runtime.permits.insert("other".into());
    assert!(!runtime.processing_allowed("other", Some(&state)));
    let other = envelope("thread", "other", Delivery::Soon);
    runtime.recovery.messages.insert(
        "other".into(),
        Recovered {
            commit: Commit {
                receipt: Receipt::new(&other, "old".into(), 1).unwrap(),
                envelope: other,
            },
            processing: Processing::Pending(None),
        },
    );
    assert!(!runtime.has_soon_continuation(&SoonBoundary {
        current: Arc::clone(&state),
        input: InputBoundary::Drain,
        accepts_mail: true,
    }));
    drop(external);
    assert!(
        matches!(session.reserve_exclusive_external_input(value, "second", true).await.unwrap(), Reservation::Known((_, Some(id))) if id == "answer-turn")
    );
}

#[tokio::test]
async fn stop_answer_refuses_open_pause_restored_and_tool_gates_without_queueing() {
    let session = session_with_runtime(runtime()).await;
    let value = answer(&session.thread_id.to_string(), "answer");
    for (paused, stopped) in [(false, false), (true, false)] {
        {
            let mut external = session.external_input.lock().await;
            let runtime = external.as_mut().unwrap();
            runtime.paused = paused;
            runtime.stopped = stopped;
        }
        assert_eq!(
            session
                .submit_exclusive_external_input(value.clone(), true)
                .await
                .unwrap(),
            Err(BusyReason::GateClosed)
        );
    }
    // A subsequent Stop cannot turn an ordinary/restored pause into a bypass.
    session.stop_external_input().await.unwrap();
    assert!(
        !session
            .external_input
            .lock()
            .await
            .as_ref()
            .unwrap()
            .stopped
    );
    {
        let mut external = session.external_input.lock().await;
        external.as_mut().unwrap().stopped = true;
    }
    session.tool_gate.set(true);
    assert_eq!(
        session
            .submit_exclusive_external_input(value.clone(), true)
            .await
            .unwrap(),
        Err(BusyReason::ToolGate)
    );
    session.tool_gate.set(false);
    // Explicit pause after Stop revokes the volatile bypass even if closed already.
    session.external_input_gate(true).await.unwrap();
    assert_eq!(
        session
            .submit_exclusive_external_input(value, true)
            .await
            .unwrap(),
        Err(BusyReason::GateClosed)
    );
    assert!(session.active_turn.lock().await.is_none());
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
}

#[derive(Debug)]
struct Draining;

impl codex_extension_api::TurnStartAdmission for Draining {
    fn admit_turn_start(&self) -> Option<Box<dyn Send>> {
        None
    }
}

#[tokio::test]
async fn stop_answer_refuses_draining_and_plan_mode() {
    let (mut session, _) = crate::session::tests::make_session_and_context().await;
    let mut runtime = runtime();
    runtime.paused = true;
    runtime.stopped = true;
    *session.external_input.lock().await = Some(runtime);
    let mut extensions = codex_extension_api::ExtensionRegistryBuilder::new();
    extensions.turn_start_admission(Arc::new(Draining));
    session.services.extensions = Arc::new(extensions.build());
    let session = Arc::new(session);
    let value = answer(&session.thread_id.to_string(), "answer");
    assert_eq!(
        session
            .submit_exclusive_external_input(value.clone(), true)
            .await
            .unwrap(),
        Err(BusyReason::ServerDraining)
    );
    {
        let mut state = session.state.lock().await;
        Arc::make_mut(&mut state.session_configuration.step_settings)
            .collaboration_mode
            .mode = ModeKind::Plan;
    }
    assert_eq!(
        session
            .submit_exclusive_external_input(value, true)
            .await
            .unwrap(),
        Err(BusyReason::PlanMode)
    );
    assert!(session.active_turn.lock().await.is_none());
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
}

#[tokio::test]
async fn patch_approval_abort_closes_external_input_without_owner_stop_provenance() {
    struct PendingPatchTask;
    impl crate::tasks::SessionTask for PendingPatchTask {
        fn kind(&self) -> crate::state::TaskKind {
            crate::state::TaskKind::Regular
        }
        fn span_name(&self) -> &'static str {
            "test.pending_patch"
        }
        async fn run(
            self: Arc<Self>,
            _session: Arc<Session>,
            _turn: Arc<crate::session::TurnContext>,
            _input: Vec<crate::session::TurnInput>,
            cancellation: tokio_util::sync::CancellationToken,
        ) -> crate::tasks::SessionTaskResult {
            cancellation.cancelled().await;
            Ok(None)
        }
    }
    let (mut session, turn, events) =
        crate::session::tests::make_session_and_context_with_rx().await;
    crate::session::tests::attach_thread_persistence(Arc::get_mut(&mut session).unwrap()).await;
    *session.external_input.lock().await = Some(runtime());
    for stopped in [false, true] {
        {
            let mut state = session.external_input.lock().await;
            let runtime = state.as_mut().unwrap();
            runtime.paused = stopped;
            runtime.stopped = stopped;
        }
        session
            .spawn_task(Arc::clone(&turn), Vec::new(), PendingPatchTask)
            .await;
        let request = tokio::spawn({
            let session = Arc::clone(&session);
            let turn = Arc::clone(&turn);
            async move {
                session
                    .request_patch_approval(
                        &turn,
                        "patch".into(),
                        std::collections::HashMap::new(),
                        None,
                        None,
                    )
                    .await
            }
        });
        while !matches!(
            events.recv().await.unwrap().msg,
            codex_protocol::protocol::EventMsg::ApplyPatchApprovalRequest(_)
        ) {}
        crate::session::handlers::cancel_patch_approval(&session, "patch", Some("stale-turn"))
            .await;
        {
            let state = session.external_input.lock().await;
            let runtime = state.as_ref().unwrap();
            assert_eq!(runtime.paused, stopped);
            assert_eq!(runtime.stopped, stopped);
        }
        crate::session::handlers::cancel_patch_approval(&session, "patch", Some(&turn.sub_id))
            .await;
        assert_eq!(
            request.await.unwrap(),
            codex_protocol::protocol::ReviewDecision::Abort
        );
        let state = session.external_input.lock().await;
        let runtime = state.as_ref().unwrap();
        assert!(runtime.paused);
        assert!(!runtime.stopped);
    }
    assert_eq!(
        session
            .submit_exclusive_external_input(answer(&session.thread_id.to_string(), "answer"), true)
            .await
            .unwrap(),
        Err(BusyReason::GateClosed)
    );
}

#[tokio::test]
async fn only_explicit_owner_interrupt_submission_carries_stop_provenance_and_trace() {
    let (session, turn, events) = crate::session::tests::make_session_and_context_with_rx().await;
    let (thread, submissions) = crate::session::tests::make_thread_for_session(
        session,
        &turn,
        events,
        codex_protocol::protocol::SessionSource::default(),
    )
    .await;
    thread
        .submit(codex_protocol::protocol::Op::Interrupt)
        .await
        .unwrap();
    let ordinary = submissions.recv().await.unwrap();
    assert!(matches!(
        ordinary.op.as_protocol(),
        Some(codex_protocol::protocol::Op::Interrupt)
    ));
    assert!(!ordinary.owner_stop);
    let trace = codex_protocol::protocol::W3cTraceContext {
        traceparent: Some("00-1234567890abcdef1234567890abcdef-1234567890abcdef-01".into()),
        tracestate: Some("test=owner".into()),
    };
    let id = thread
        .submit_owner_interrupt_with_trace(Some(trace.clone()))
        .await
        .unwrap();
    let owner = submissions.recv().await.unwrap();
    assert_eq!(owner.id, id);
    assert!(matches!(
        owner.op.as_protocol(),
        Some(codex_protocol::protocol::Op::Interrupt)
    ));
    assert!(owner.owner_stop);
    assert_eq!(owner.trace, Some(trace));
}
