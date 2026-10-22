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
        paused: false,
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
        .submit_exclusive_external_input(value.clone())
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
            .submit_exclusive_external_input(value)
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
