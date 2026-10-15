//! Forking a paginated thread whose ExternalInput ledger spans a compaction.
//!
//! A reference-backed fork hands core only the bounded model context (from the newest
//! compaction on). The ExternalInput check must read the complete source rollout instead:
//! the suffix starts mid-ledger and used to be refused as "partial or corrupt".
use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use codex_core::CodexThread;
use codex_history::InitialHistory;
use codex_history::ResumedHistory;
use codex_protocol::mcp::ClientMcpExtensions;
use codex_core::context::CanonicalBytePolicy;
use codex_protocol::external_input::Delivery;
use codex_protocol::external_input::Envelope;
use codex_protocol::external_input::Message;
use codex_protocol::external_input::Source;
use codex_protocol::external_input::SourceKind;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadHistoryMode;
use codex_thread_store::ForkBoundary;
use codex_thread_store::LoadThreadHistoryParams;
use codex_thread_store::PrepareForkParams;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use wiremock::MockServer;

const OWNER: &str = "cutex-owner";

/// A paginated thread resumed under the trusted launch binding, as cutex does. Paginated threads
/// resume from their latest model context (not a rollout path), like the compression test.
struct Bound {
    test: TestCodex,
    thread: Arc<CodexThread>,
    thread_id: codex_protocol::ThreadId,
}

async fn bound_paginated_thread(server: &MockServer) -> Result<Bound> {
    let local_compaction = |config: &mut codex_core::config::Config| {
        config.model_provider.name = "Local compaction test provider".to_string();
    };
    let initial = test_codex()
        .with_history_mode(ThreadHistoryMode::Paginated)
        .with_config(local_compaction)
        .build(server)
        .await?;
    initial.codex.ensure_rollout_materialized().await;
    initial.codex.flush_rollout().await?;
    let thread_id = initial.session_configured.thread_id;
    let rollout_path = initial.codex.rollout_path().context("initial rollout")?;
    initial.codex.shutdown_and_wait().await?;
    let owner_thread = thread_id.to_string();
    let test = test_codex()
        .with_history_mode(ThreadHistoryMode::Paginated)
        .with_config(local_compaction)
        .with_home(Arc::clone(&initial.home))
        .with_thread_manager(move |manager| {
            manager.with_external_input_owner(
                OWNER.to_string(),
                owner_thread,
                CanonicalBytePolicy::default(),
            )
        })
        .build(server)
        .await?;
    let context = test
        .thread_store
        .load_latest_model_context(LoadThreadHistoryParams {
            thread_id,
            include_archived: false,
        })
        .await?;
    let resumed = test
        .thread_manager
        .resume_thread_with_history(
            test.config.clone(),
            InitialHistory::Resumed(ResumedHistory {
                conversation_id: context.thread_id,
                history: Arc::new(context.items),
                rollout_path: Some(rollout_path),
            }),
            codex_core::test_support::auth_manager_from_auth(codex_login::CodexAuth::from_api_key(
                "dummy",
            )),
            /*parent_trace*/ None,
            ClientMcpExtensions::default(),
        )
        .await?;
    Ok(Bound {
        test,
        thread: resumed.thread,
        thread_id,
    })
}

fn envelope(thread_id: codex_protocol::ThreadId, id: &str) -> Envelope {
    let mut envelope = Envelope {
        version: 1,
        owner_id: OWNER.to_string(),
        thread_id: thread_id.to_string(),
        runtime_generation: 1,
        message: Message {
            id: id.to_string(),
            source: Source {
                kind: SourceKind::Agent,
                id: "requester".to_string(),
            },
            event_type: "ASK".to_string(),
            delivery: Delivery::AfterTurn,
            text: format!("payload-{id}"),
        },
        semantic_sha256: String::new(),
        view: None,
    };
    envelope.semantic_sha256 = envelope.digest();
    envelope
}

async fn external_turn(server: &MockServer, bound: &Bound, id: &str) -> Result<()> {
    mount_sse_once(
        server,
        sse(vec![
            ev_response_created(id),
            ev_assistant_message(&format!("{id}-message"), "done"),
            ev_completed(id),
        ]),
    )
    .await;
    bound
        .thread
        .submit_external_input_exclusive(envelope(bound.thread_id, id))
        .await?
        .expect("exclusive admission starts a turn");
    wait_for_event(&bound.thread, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fork_after_compaction_validates_the_complete_external_input_ledger() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = start_mock_server().await;
    let bound = bound_paginated_thread(&server).await?;
    external_turn(&server, &bound, "before-compaction").await?;
    mount_sse_once(
        &server,
        sse(vec![
            ev_assistant_message("summary", "COMPACTION_SUMMARY"),
            ev_completed("compact"),
        ]),
    )
    .await;
    bound.thread.submit(Op::Compact).await?;
    wait_for_event(&bound.thread, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    external_turn(&server, &bound, "after-compaction").await?;
    bound.thread.flush_rollout().await?;

    let test = &bound.test;
    let prepared = test
        .thread_store
        .prepare_fork(PrepareForkParams {
            thread_id: bound.thread_id,
            boundary: ForkBoundary::Latest,
        })
        .await?;
    assert!(
        prepared.source_rollout_path.is_some(),
        "a local paginated fork names its complete source rollout"
    );
    let child = test
        .thread_manager
        .fork_prepared_thread(
            codex_core::StartThreadOptions::new(test.config.clone()),
            prepared,
        )
        .await?;
    child.thread.shutdown_and_wait().await?;
    bound.thread.shutdown_and_wait().await?;
    test.codex.shutdown_and_wait().await?;
    Ok(())
}
