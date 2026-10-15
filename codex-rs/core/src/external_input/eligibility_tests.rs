use super::*;
use crate::external_input::Pending;
use crate::external_input::Runtime;
use codex_protocol::external_input::*;
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

fn pending(delivery: Delivery) -> Pending {
    let mut envelope = Envelope {
        view: None,
        version: 1,
        owner_id: "owner".into(),
        thread_id: "thread".into(),
        runtime_generation: 7,
        message: Message {
            id: "message".into(),
            source: Source {
                kind: SourceKind::Agent,
                id: "agent".into(),
            },
            event_type: "opaque".into(),
            delivery,
            text: "data".into(),
        },
        semantic_sha256: String::new(),
    };
    envelope.semantic_sha256 = envelope.digest();
    Pending {
        envelope,
        excluded_turn: None,
        running_turn: None,
        exclusive_turn: None,
    }
}

#[test]
fn reserved_initial_running_drain_and_compaction_boundaries() {
    let runtime = runtime();
    let mut pending = pending(Delivery::Soon);
    let mut boundary = SoonBoundary {
        current: Arc::new(Mutex::new(TurnState::default())),
        input: InputBoundary::Initial,
        accepts_mail: true,
    };
    // Idle/reserved admission can join first direct input. Installed-task
    // admission waits until that input has been sampled, then a real drain.
    assert!(runtime.can_consume_soon(&pending, &boundary));
    pending.running_turn = Some(Arc::clone(&boundary.current));
    assert!(!runtime.can_consume_soon(&pending, &boundary));
    boundary.input = InputBoundary::Drain;
    assert!(runtime.can_consume_soon(&pending, &boundary));
    boundary.input = InputBoundary::Deferred;
    assert!(!runtime.can_consume_soon(&pending, &boundary));
    boundary.input = InputBoundary::Initial;
    boundary.current = Arc::new(Mutex::new(TurnState::default()));
    assert!(runtime.can_consume_soon(&pending, &boundary));
}

#[test]
fn continuation_uses_consumer_predicate_and_preserves_holds() {
    let mut runtime = runtime();
    runtime.pending.push(pending(Delivery::Soon));
    let mut boundary = SoonBoundary {
        current: Arc::new(Mutex::new(TurnState::default())),
        input: InputBoundary::Drain,
        accepts_mail: true,
    };
    assert!(runtime.has_soon_continuation(&boundary));
    boundary.accepts_mail = false;
    assert!(!runtime.has_soon_continuation(&boundary));
    boundary.accepts_mail = true;
    runtime.paused = true;
    assert!(!runtime.has_soon_continuation(&boundary));
    runtime.paused = false;
    runtime.poisoned = true;
    assert!(!runtime.has_soon_continuation(&boundary));
    runtime.poisoned = false;
    runtime.policy_blocked.insert("message".into());
    assert!(!runtime.has_soon_continuation(&boundary));
    runtime.policy_blocked.clear();
    for mode in [Delivery::AfterTurn, Delivery::Passive] {
        runtime.pending[0] = pending(mode);
        assert!(!runtime.has_soon_continuation(&boundary));
    }
}

#[test]
fn retry_permit_excludes_current_turn_and_never_releases_other_work() {
    let mut runtime = runtime();
    let value = pending(Delivery::Soon).envelope;
    let commit = Commit {
        receipt: Receipt::new(&value, "old-turn".into(), 1).unwrap(),
        envelope: value,
    };
    runtime.recovery.messages.insert(
        "message".into(),
        Recovered {
            commit,
            processing: Processing::Pending(None),
        },
    );
    runtime.pending.push(pending(Delivery::Soon));
    runtime.paused = true;
    let mut boundary = SoonBoundary {
        current: Arc::new(Mutex::new(TurnState::default())),
        input: InputBoundary::Drain,
        accepts_mail: true,
    };
    assert!(!runtime.has_soon_continuation(&boundary));
    runtime.permits.insert("message".into());
    runtime
        .retry_exclusions
        .insert("message".into(), Arc::clone(&boundary.current));
    assert!(!runtime.has_soon_continuation(&boundary));
    boundary.current = Arc::new(Mutex::new(TurnState::default()));
    assert!(runtime.has_soon_continuation(&boundary));
    assert!(!runtime.can_consume_soon(&runtime.pending[0], &boundary));
    for processing in [
        Processing::Held(None, HoldReason::NoOutput),
        Processing::Held(None, HoldReason::RequestUncertain),
        Processing::OutputObserved(uuid::Uuid::nil()),
    ] {
        runtime
            .recovery
            .messages
            .get_mut("message")
            .unwrap()
            .processing = processing;
        assert!(!runtime.has_soon_continuation(&boundary));
    }
}

#[tokio::test]
async fn reserved_turn_admission_keeps_identity_without_running_deferral() {
    let (session, _) = crate::session::tests::make_session_and_context().await;
    *session.external_input.lock().await = Some(runtime());
    let reserved = crate::state::ActiveTurn::default();
    let identity = Arc::clone(&reserved.turn_state);
    *session.active_turn.lock().await = Some(reserved);
    let mut value = pending(Delivery::Soon).envelope;
    value.thread_id = session.thread_id.to_string();
    value.semantic_sha256 = value.digest();
    session.admit_external_input(value).await.unwrap();
    let state = session.external_input.lock().await;
    let runtime = state.as_ref().unwrap();
    let admitted = &runtime.pending[0];
    assert!(Arc::ptr_eq(
        admitted.excluded_turn.as_ref().unwrap(),
        &identity
    ));
    assert!(admitted.running_turn.is_none());
    let boundary = SoonBoundary {
        current: identity,
        input: InputBoundary::Initial,
        accepts_mail: true,
    };
    assert!(runtime.can_consume_soon(admitted, &boundary));
}
