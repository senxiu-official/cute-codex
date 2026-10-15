//! Exclusive ExternalInput admission and gate control on a bound, resumed thread.
use anyhow::Result;
use codex_core::context::CanonicalBytePolicy;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Message;
use codex_protocol::external_input::Source;
use codex_protocol::external_input::SourceKind;
use codex_protocol::external_input_status::BusyReason;
use codex_protocol::external_input_status::DeliveryState;
use codex_protocol::external_input_status::ProcessingState;
use codex_protocol::external_input_status::Status;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use core_test_support::responses::ResponseMock;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_response_once;
use core_test_support::responses::received_responses_requests;
use core_test_support::responses::sse;
use core_test_support::responses::sse_response;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use std::time::Duration;
use wiremock::MockServer;

const OWNER: &str = "cutex-owner";

/// Restarts a fresh thread with the trusted launch binding, as cutex does.
async fn bound_thread(server: &MockServer) -> Result<TestCodex> {
    let initial = test_codex().build(server).await?;
    initial.codex.ensure_rollout_materialized().await;
    initial.codex.flush_rollout().await?;
    let thread = initial.session_configured.thread_id.to_string();
    test_codex()
        .with_thread_manager(move |manager| {
            manager.with_external_input_owner(
                OWNER.to_string(),
                thread,
                CanonicalBytePolicy::default(),
            )
        })
        .restart(server, &initial)
        .await
}

fn envelope(test: &TestCodex, id: &str, delivery: Delivery) -> Envelope {
    let mut envelope = Envelope {
        version: 1,
        owner_id: OWNER.to_string(),
        thread_id: test.session_configured.thread_id.to_string(),
        runtime_generation: 1,
        message: Message {
            id: id.to_string(),
            source: Source {
                kind: SourceKind::Agent,
                id: "requester".to_string(),
            },
            event_type: "ASK".to_string(),
            delivery,
            text: format!("payload-{id}"),
        },
        semantic_sha256: String::new(),
        view: None,
    };
    envelope.semantic_sha256 = envelope.digest();
    envelope
}

async fn status(test: &TestCodex, envelope: &Envelope) -> Status {
    test.codex
        .external_input_status(OWNER, &envelope.message.id, &envelope.semantic_sha256)
        .await
        .expect("status")
}

fn reply(id: &str, delay: Duration) -> wiremock::ResponseTemplate {
    sse_response(sse(vec![
        ev_response_created(id),
        ev_assistant_message(&format!("{id}-message"), "done"),
        ev_completed(id),
    ]))
    .set_delay(delay)
}

async fn exclusive(test: &TestCodex, envelope: &Envelope) -> String {
    let (status, turn) = test
        .codex
        .submit_external_input_exclusive(envelope.clone())
        .await
        .expect("admission decision")
        .expect("exclusive admission starts a turn");
    assert_eq!(status.delivery_state, DeliveryState::Pending);
    turn.expect("started turn id")
}

async fn wait_for_requests(mock: &ResponseMock, count: usize) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while mock.requests().len() < count {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("model request");
}

async fn turn_complete(test: &TestCodex) -> String {
    match wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await
    {
        EventMsg::TurnComplete(event) => event.turn_id,
        _ => unreachable!(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exclusive_turn_carries_only_its_item_and_binds_the_receipt() -> Result<()> {
    let server = start_mock_server().await;
    let test = bound_thread(&server).await?;
    mount_response_once(&server, reply("first", Duration::from_millis(500))).await;
    mount_response_once(&server, reply("second", Duration::ZERO)).await;

    let ask = envelope(&test, "ask-1", Delivery::AfterTurn);
    let turn_id = exclusive(&test, &ask).await;
    // Turn-triggering input admitted while the exclusive turn runs waits for
    // its own turn, even `soon` input that would otherwise join this one.
    let other = envelope(&test, "other", Delivery::Soon);
    test.codex.submit_external_input(other.clone()).await?;
    // A busy thread refuses a second exclusive item without queueing it.
    let refused = envelope(&test, "ask-2", Delivery::AfterTurn);
    assert_eq!(
        test.codex
            .submit_external_input_exclusive(refused.clone())
            .await?,
        Err(BusyReason::ActiveTurn)
    );
    assert_eq!(
        status(&test, &refused).await.delivery_state,
        DeliveryState::Unknown
    );

    assert_eq!(turn_complete(&test).await, turn_id);
    let ask_status = status(&test, &ask).await;
    assert_eq!(ask_status.delivery_state, DeliveryState::ContextPersisted);
    assert_eq!(
        ask_status.receipt.as_ref().map(|r| r.turn_id.as_str()),
        Some(turn_id.as_str())
    );
    assert_eq!(ask_status.processing.state, ProcessingState::OutputObserved);
    let requests = received_responses_requests(&server).await;
    let request = &requests[0];
    assert!(request.body_contains_text("payload-ask-1"));
    assert!(!request.body_contains_text("payload-other"));

    let next_turn = turn_complete(&test).await;
    assert_ne!(next_turn, turn_id);
    let other_status = status(&test, &other).await;
    assert_eq!(
        other_status.receipt.as_ref().map(|r| r.turn_id.as_str()),
        Some(next_turn.as_str())
    );
    let requests = received_responses_requests(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(requests[1].body_contains_text("payload-other"));
    assert_eq!(
        status(&test, &refused).await.delivery_state,
        DeliveryState::Unknown
    );

    // Exact replay reports the bound turn and starts nothing.
    assert_eq!(
        test.codex
            .submit_external_input_exclusive(ask.clone())
            .await?,
        Ok((ask_status, Some(turn_id)))
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interrupt_keeping_external_input_leaves_the_gate_open() -> Result<()> {
    let server = start_mock_server().await;
    let test = bound_thread(&server).await?;
    let stalled = mount_response_once(&server, reply("stalled", Duration::from_secs(60))).await;
    mount_response_once(&server, reply("after", Duration::ZERO)).await;

    let stopped = envelope(&test, "stopped", Delivery::AfterTurn);
    let turn_id = exclusive(&test, &stopped).await;
    wait_for_requests(&stalled, 1).await;
    assert!(
        test.codex
            .interrupt_turn_keeping_external_input(&turn_id)
            .await
    );
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;
    assert!(
        !test
            .codex
            .interrupt_turn_keeping_external_input(&turn_id)
            .await
    );
    // The stopped attempt is held for the requester; it does not block admission.
    assert_eq!(
        status(&test, &stopped).await.processing.state,
        ProcessingState::Held
    );

    let next = envelope(&test, "next", Delivery::AfterTurn);
    let next_turn = exclusive(&test, &next).await;
    assert_eq!(turn_complete(&test).await, next_turn);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_interrupt_closes_the_gate_until_resume() -> Result<()> {
    let server = start_mock_server().await;
    let test = bound_thread(&server).await?;
    let stalled = mount_response_once(&server, reply("stalled", Duration::from_secs(60))).await;
    mount_response_once(&server, reply("resumed", Duration::ZERO)).await;

    let first = envelope(&test, "first", Delivery::AfterTurn);
    exclusive(&test, &first).await;
    wait_for_requests(&stalled, 1).await;
    test.codex.submit(Op::Interrupt).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;

    let ask = envelope(&test, "ask", Delivery::AfterTurn);
    assert_eq!(
        test.codex
            .submit_external_input_exclusive(ask.clone())
            .await?,
        Err(BusyReason::GateClosed)
    );
    assert_eq!(
        status(&test, &ask).await.delivery_state,
        DeliveryState::Unknown
    );

    assert!(test.codex.resume_external_input().await?);
    assert!(!test.codex.resume_external_input().await?);
    let turn_id = exclusive(&test, &ask).await;
    assert_eq!(turn_complete(&test).await, turn_id);
    Ok(())
}
