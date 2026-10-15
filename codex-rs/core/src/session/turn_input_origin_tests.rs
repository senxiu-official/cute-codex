use super::*;
use crate::external_input::Runtime;
use crate::session::tests::make_session_and_context;
use codex_protocol::external_input::recover;
use pretty_assertions::assert_eq;

fn runtime() -> Runtime {
    Runtime {
        policy: Default::default(),
        policy_blocked: Default::default(),
        dispatch_revision: 0,
        attempted_revision: 0,
        retry_exclusions: Default::default(),
        changed: tokio::sync::broadcast::channel(100).0,
        owner: "owner".into(),
        recovery: recover("owner", "thread", &[]).unwrap(),
        pending: Vec::new(),
        paused: true,
        poisoned: false,
        retries: Default::default(),
        permits: Default::default(),
        exclusive_turn: None,
    }
}

#[tokio::test]
async fn only_explicit_origin_can_reserve_while_paused() {
    let (session, _) = make_session_and_context().await;
    *session.external_input.lock().await = Some(runtime());
    assert!(matches!(
        reserve_idle_turn(
            &session,
            IdleReservation::Origin(TurnStartOrigin::Automatic),
            /*expected_previous_turn_id*/ None
        )
        .await,
        Err(NotSubmittedReason::Interrupted)
    ));
    assert!(session.active_turn.lock().await.is_none());
    assert!(
        reserve_idle_turn(
            &session,
            IdleReservation::Origin(TurnStartOrigin::Explicit),
            /*expected_previous_turn_id*/ None
        )
        .await
        .is_ok()
    );
    // Reservation itself never silently clears a durable gate.
    assert!(session.external_input.lock().await.as_ref().unwrap().paused);
}

#[tokio::test]
#[expect(
    clippy::await_holding_invalid_type,
    reason = "deterministically exercise reservation lock ordering"
)]
async fn interruption_cannot_pass_between_gate_check_and_reservation() {
    let (session, _) = make_session_and_context().await;
    let mut state = runtime();
    state.paused = false;
    *session.external_input.lock().await = Some(state);
    let active_guard = session.active_turn.lock().await;
    let reservation = reserve_idle_turn(
        &session,
        IdleReservation::Origin(TurnStartOrigin::Automatic),
        /*expected_previous_turn_id*/ None,
    );
    tokio::pin!(reservation);
    assert!(futures::poll!(&mut reservation).is_pending());
    // The reservation is waiting for active_turn while retaining the gate lock.
    assert!(session.external_input.try_lock().is_err());
    drop(active_guard);
    assert!(reservation.await.is_ok());
    let mut gate = session.external_input.lock().await;
    gate.as_mut().unwrap().paused = true;
    drop(gate);
    assert!(session.active_turn.lock().await.is_some());
}

#[tokio::test]
async fn automatic_unbound_reservation_preserves_upstream_behavior() {
    let (session, _) = make_session_and_context().await;
    assert!(
        reserve_idle_turn(
            &session,
            IdleReservation::Origin(TurnStartOrigin::Automatic),
            /*expected_previous_turn_id*/ None
        )
        .await
        .is_ok()
    );
    assert!(matches!(
        reserve_idle_turn(
            &session,
            IdleReservation::Origin(TurnStartOrigin::Explicit),
            /*expected_previous_turn_id*/ None
        )
        .await,
        Err(NotSubmittedReason::NotIdle)
    ));
}

#[test]
fn trigger_text_cannot_select_explicit_origin() {
    let request = TurnInputRequest::user_input(vec![])
        .automatic_start()
        .on_start(TurnStartOptions {
            turn_trigger: Some("human".into()),
            ..Default::default()
        });
    assert_eq!(request.start_origin(), TurnStartOrigin::Automatic);
    assert_eq!(
        TurnInputRequest::user_input(vec![]).start_origin(),
        TurnStartOrigin::Explicit
    );
}

#[tokio::test]
async fn external_reservation_rechecks_work_under_the_gate_lock() {
    let (session, _) = make_session_and_context().await;
    *session.external_input.lock().await = Some(runtime());
    assert!(matches!(
        reserve_idle_turn(
            &session,
            IdleReservation::ExternalInput,
            /*expected_previous_turn_id*/ None
        )
        .await,
        Err(NotSubmittedReason::Interrupted)
    ));
    assert!(session.active_turn.lock().await.is_none());
}

#[tokio::test]
async fn poisoned_external_input_log_does_not_prevent_interrupting_reserved_turn() {
    let (session, _) = make_session_and_context().await;
    reserve_idle_turn(
        &session,
        IdleReservation::Origin(TurnStartOrigin::Explicit),
        /*expected_previous_turn_id*/ None,
    )
    .await
    .unwrap();
    let mut gate = runtime();
    gate.poisoned = true;
    *session.external_input.lock().await = Some(gate);
    let session = Arc::new(session);
    session.interrupt_task().await;
    assert!(session.active_turn.lock().await.is_none());
    assert!(
        session
            .external_input
            .lock()
            .await
            .as_ref()
            .unwrap()
            .poisoned
    );
    assert!(matches!(
        reserve_idle_turn(
            &session,
            IdleReservation::Origin(TurnStartOrigin::Automatic),
            /*expected_previous_turn_id*/ None
        )
        .await,
        Err(NotSubmittedReason::Interrupted)
    ));
}

#[tokio::test]
async fn superseded_turn_does_not_reserve_or_clear_external_gate() {
    let (session, _) = make_session_and_context().await;
    *session.external_input.lock().await = Some(runtime());
    assert!(matches!(
        reserve_idle_turn(
            &session,
            IdleReservation::Origin(TurnStartOrigin::Explicit),
            Some("stale-turn"),
        )
        .await,
        Err(NotSubmittedReason::Superseded)
    ));
    assert!(session.active_turn.lock().await.is_none());
    assert!(session.external_input.lock().await.as_ref().unwrap().paused);
}
