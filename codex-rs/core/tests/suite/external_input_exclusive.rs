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

fn answer(test: &TestCodex, id: &str) -> Envelope {
    let mut value = envelope(test, id, Delivery::AfterTurn);
    value.message.source = Source {
        kind: SourceKind::Service,
        id: "cutex-asks".into(),
    };
    value.message.event_type = "ANSWER".into();
    value.semantic_sha256 = value.digest();
    value
}

async fn owner_stop_when_idle(test: &TestCodex) -> Result<()> {
    test.codex.submit_owner_interrupt_with_trace(None).await?;
    test.codex
        .update_thread_settings(codex_protocol::protocol::ThreadSettingsOverrides::default())
        .await?;
    Ok(())
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
    test.codex.submit_owner_interrupt_with_trace(None).await?;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_answer_runs_once_without_resuming_or_consuming_other_queued_input() -> Result<()> {
    let server = start_mock_server().await;
    let test = bound_thread(&server).await?;
    let stalled = mount_response_once(&server, reply("stalled", Duration::from_secs(60))).await;
    let answer_request =
        mount_response_once(&server, reply("answer", Duration::from_millis(500))).await;
    mount_response_once(&server, reply("resumed", Duration::ZERO)).await;
    let first = envelope(&test, "first", Delivery::AfterTurn);
    exclusive(&test, &first).await;
    wait_for_requests(&stalled, 1).await;
    test.codex.submit_owner_interrupt_with_trace(None).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;

    let mut others = vec![
        envelope(&test, "queued-soon", Delivery::Soon),
        envelope(&test, "queued-after", Delivery::AfterTurn),
        envelope(&test, "queued-passive", Delivery::Passive),
    ];
    for other in &others {
        test.codex.submit_external_input(other.clone()).await?;
    }
    let mut answer = envelope(&test, "answer", Delivery::AfterTurn);
    answer.message.source = Source {
        kind: SourceKind::Service,
        id: "cutex-asks".into(),
    };
    answer.message.event_type = "ANSWER".into();
    answer.semantic_sha256 = answer.digest();
    let (_, turn_id) = test
        .codex
        .submit_external_input_stop_answer(answer.clone())
        .await?
        .expect("Stop ANSWER accepted");
    let turn_id = turn_id.expect("dedicated answer turn");
    for (id, delivery) in [
        ("arriving-soon", Delivery::Soon),
        ("arriving-passive", Delivery::Passive),
    ] {
        let other = envelope(&test, id, delivery);
        test.codex.submit_external_input(other.clone()).await?;
        others.push(other);
    }
    // A replay while pending/claimed cannot start a second turn.
    let replay = test
        .codex
        .submit_external_input_stop_answer(answer.clone())
        .await?
        .expect("known answer");
    assert_eq!(replay.1.as_deref(), Some(turn_id.as_str()));
    wait_for_requests(&answer_request, 1).await;
    assert_eq!(turn_complete(&test).await, turn_id);
    let completed = status(&test, &answer).await;
    assert_eq!(completed.delivery_state, DeliveryState::ContextPersisted);
    assert_eq!(completed.processing.state, ProcessingState::OutputObserved);
    assert_eq!(
        completed
            .receipt
            .as_ref()
            .map(|receipt| receipt.turn_id.as_str()),
        Some(turn_id.as_str())
    );
    for other in &others {
        assert_eq!(
            status(&test, other).await.delivery_state,
            DeliveryState::Pending
        );
    }
    let requests = received_responses_requests(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(requests[1].body_contains_text("payload-answer"));
    for other in &others {
        assert!(!requests[1].body_contains_text(&other.message.text));
    }
    let ordinary = envelope(&test, "ordinary", Delivery::AfterTurn);
    assert_eq!(
        test.codex.submit_external_input_exclusive(ordinary).await?,
        Err(BusyReason::GateClosed)
    );
    // Exact replay returns the original receipt even behind a new blocker.
    test.codex.set_tool_gate(true);
    assert_eq!(
        test.codex.submit_external_input_stop_answer(answer).await?,
        Ok((completed, Some(turn_id)))
    );
    test.codex.set_tool_gate(false);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            wait_for_event(&test.codex, |event| matches!(
                event,
                EventMsg::TurnStarted(_)
            ))
        )
        .await
        .is_err(),
        "no automatic turn after ANSWER"
    );
    assert!(
        test.codex.resume_external_input().await?,
        "gate stayed closed throughout ANSWER"
    );
    turn_complete(&test).await;
    for other in &others {
        assert_eq!(
            status(&test, other).await.delivery_state,
            DeliveryState::ContextPersisted
        );
    }
    assert_eq!(received_responses_requests(&server).await.len(), 3);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_answer_replay_after_stop_reports_held_without_a_turn() -> Result<()> {
    let server = start_mock_server().await;
    let test = bound_thread(&server).await?;
    let stalled = mount_response_once(&server, reply("stalled", Duration::from_secs(60))).await;
    exclusive(&test, &envelope(&test, "first", Delivery::AfterTurn)).await;
    wait_for_requests(&stalled, 1).await;
    let value = answer(&test, "queued-answer");
    test.codex.submit_external_input(value.clone()).await?;
    test.codex.submit_owner_interrupt_with_trace(None).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;
    let held = status(&test, &value).await;
    assert_eq!(held.delivery_state, DeliveryState::Pending);
    assert_eq!(held.processing.state, ProcessingState::Held);
    assert_eq!(
        held.processing.reason,
        Some(codex_protocol::external_input::HoldReason::Interrupted)
    );
    for _ in 0..2 {
        assert_eq!(
            test.codex
                .submit_external_input_stop_answer(value.clone())
                .await?,
            Ok((held.clone(), None))
        );
    }
    assert_eq!(received_responses_requests(&server).await.len(), 1);
    mount_response_once(&server, reply("resumed", Duration::ZERO)).await;
    assert!(test.codex.resume_external_input().await?);
    let resumed = turn_complete(&test).await;
    assert_eq!(
        status(&test, &value).await.receipt.unwrap().turn_id,
        resumed
    );
    assert_eq!(received_responses_requests(&server).await.len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_resume_during_stop_answer_keeps_its_turn_exclusive_then_dispatches_queue()
-> Result<()> {
    let server = start_mock_server().await;
    let test = bound_thread(&server).await?;
    owner_stop_when_idle(&test).await?;
    let running = mount_response_once(&server, reply("answer", Duration::from_secs(5))).await;
    mount_response_once(&server, reply("queued", Duration::ZERO)).await;
    let queued = envelope(&test, "queued-soon", Delivery::Soon);
    test.codex.submit_external_input(queued.clone()).await?;
    let value = answer(&test, "answer");
    let (_, turn) = test
        .codex
        .submit_external_input_stop_answer(value.clone())
        .await?
        .expect("start answer");
    let turn = turn.unwrap();
    wait_for_requests(&running, 1).await;
    assert!(test.codex.resume_external_input().await?);
    let passive = envelope(&test, "passive-after-resume", Delivery::Passive);
    test.codex.submit_external_input(passive.clone()).await?;
    assert_eq!(
        status(&test, &queued).await.delivery_state,
        DeliveryState::Pending
    );
    assert_eq!(
        status(&test, &passive).await.delivery_state,
        DeliveryState::Pending
    );
    assert_eq!(received_responses_requests(&server).await.len(), 1);
    assert_eq!(turn_complete(&test).await, turn);
    let next = turn_complete(&test).await;
    assert_ne!(next, turn);
    assert_eq!(status(&test, &value).await.receipt.unwrap().turn_id, turn);
    for other in [&queued, &passive] {
        assert_eq!(status(&test, other).await.receipt.unwrap().turn_id, next);
    }
    let requests = received_responses_requests(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].body_contains_text(&queued.message.text));
    assert!(!requests[0].body_contains_text(&passive.message.text));
    assert!(requests[1].body_contains_text(&queued.message.text));
    assert!(requests[1].body_contains_text(&passive.message.text));
    assert!(
        !test.codex.resume_external_input().await?,
        "explicit owner resume remains effective"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_stop_during_stop_answer_aborts_only_its_attempt_and_keeps_queue_closed() -> Result<()>
{
    let server = start_mock_server().await;
    let test = bound_thread(&server).await?;
    owner_stop_when_idle(&test).await?;
    let stalled =
        mount_response_once(&server, reply("stalled-answer", Duration::from_secs(60))).await;
    mount_response_once(&server, reply("fresh-answer", Duration::ZERO)).await;
    let value = answer(&test, "stopped-answer");
    let (_, first_turn) = test
        .codex
        .submit_external_input_stop_answer(value.clone())
        .await?
        .expect("start answer");
    let first_turn = first_turn.unwrap();
    wait_for_requests(&stalled, 1).await;
    let queued = envelope(&test, "queued", Delivery::Soon);
    test.codex.submit_external_input(queued.clone()).await?;
    test.codex.submit_owner_interrupt_with_trace(None).await?;
    let stopped = wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;
    let EventMsg::TurnAborted(stopped) = stopped else {
        unreachable!()
    };
    assert_eq!(stopped.turn_id.as_deref(), Some(first_turn.as_str()));
    let held = status(&test, &value).await;
    assert_eq!(held.processing.state, ProcessingState::Held);
    assert_eq!(held.receipt.as_ref().unwrap().turn_id, first_turn);
    assert_eq!(
        test.codex.submit_external_input_stop_answer(value).await?,
        Ok((held, Some(first_turn.clone())))
    );
    assert_eq!(
        status(&test, &queued).await.delivery_state,
        DeliveryState::Pending
    );
    assert_eq!(
        test.codex
            .submit_external_input_exclusive(envelope(&test, "ordinary", Delivery::AfterTurn))
            .await?,
        Err(BusyReason::GateClosed)
    );
    let (_, next) = test
        .codex
        .submit_external_input_stop_answer(answer(&test, "fresh-answer"))
        .await?
        .expect("second explicit ANSWER can pass owner Stop");
    let next = next.unwrap();
    assert_ne!(next, first_turn);
    assert_eq!(turn_complete(&test).await, next);
    assert_eq!(
        status(&test, &queued).await.delivery_state,
        DeliveryState::Pending
    );
    let requests = received_responses_requests(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].body_contains_text(&queued.message.text));
    assert_eq!(
        test.codex
            .submit_external_input_exclusive(envelope(&test, "still-closed", Delivery::AfterTurn))
            .await?,
        Err(BusyReason::GateClosed)
    );
    Ok(())
}
