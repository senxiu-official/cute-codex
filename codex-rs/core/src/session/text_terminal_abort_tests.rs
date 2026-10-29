use super::*;
use crate::session::TurnInput;
use crate::state::TaskKind;
use crate::tasks::SessionTask;
use crate::tasks::SessionTaskResult;
use codex_protocol::protocol::ItemCompletedEvent;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
enum AbortWindow {
    OpenItem,
    CommittingTerminal,
    RawRecordedBeforeStart,
    AuthoritativeAfterStream,
    RawCommitInFlight,
}

struct StuckTextTask {
    window: AbortWindow,
    ready: Arc<Notify>,
    release: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}

struct MarkDropped(Arc<AtomicBool>);
impl Drop for MarkDropped {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl SessionTask for StuckTextTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }
    fn span_name(&self) -> &'static str {
        "test.text_terminal_abort"
    }
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "test deliberately stalls a text commit across await to exercise forced abort"
    )]
    async fn run(
        self: Arc<Self>,
        session: Arc<Session>,
        turn: Arc<TurnContext>,
        _input: Vec<TurnInput>,
        _cancellation: CancellationToken,
    ) -> SessionTaskResult {
        let _dropped = MarkDropped(Arc::clone(&self.dropped));
        let mut item = message("forced-item");
        if !matches!(self.window, AbortWindow::RawRecordedBeforeStart) {
            session.emit_turn_item_started(&turn, &item).await;
            session
                .send_event(
                    &turn,
                    EventMsg::AgentMessageContentDelta(AgentMessageContentDeltaEvent {
                        thread_id: session.thread_id.to_string(),
                        turn_id: turn.sub_id.clone(),
                        item_id: item.id(),
                        delta: "kept 中文🙂".into(),
                    }),
                )
                .await;
        }
        match self.window {
            AbortWindow::OpenItem => self.ready.notify_one(),
            AbortWindow::RawRecordedBeforeStart | AbortWindow::AuthoritativeAfterStream => {
                if let TurnItem::AgentMessage(message) = &mut item {
                    message.content = vec![AgentMessageContent::Text {
                        text: "kept authoritative 中文🙂".into(),
                    }];
                }
                turn.open_text_items.lock().await.stage(&item);
                let raw = codex_protocol::models::ResponseItem::Message {
                    id: Some(codex_protocol::ResponseItemId::from_server(item.id())),
                    role: "assistant".into(),
                    content: vec![codex_protocol::models::ContentItem::OutputText {
                        annotations: None,
                        logprobs: None,
                        text: "kept authoritative 中文🙂".into(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                    encrypted_content: None,
                    status: None,
                };
                session
                    .record_conversation_items(&turn, turn.model_info(), &[raw])
                    .await;
                self.ready.notify_one();
            }
            AbortWindow::RawCommitInFlight => {
                let _commit = turn.text_terminal_commit.lock().await;
                if let TurnItem::AgentMessage(message) = &mut item {
                    message.content = vec![AgentMessageContent::Text {
                        text: "kept authoritative 中文🙂".into(),
                    }];
                }
                turn.open_text_items.lock().await.stage(&item);
                self.ready.notify_one();
                self.release.notified().await;
                let raw = codex_protocol::models::ResponseItem::Message {
                    id: Some(codex_protocol::ResponseItemId::from_server(item.id())),
                    role: "assistant".into(),
                    content: vec![codex_protocol::models::ContentItem::OutputText {
                        annotations: None,
                        logprobs: None,
                        text: "kept authoritative 中文🙂".into(),
                    }],
                    phase: None,
                    internal_chat_message_metadata_passthrough: None,
                    encrypted_content: None,
                    status: None,
                };
                session
                    .record_conversation_items(&turn, turn.model_info(), &[raw])
                    .await;
                session
                    .emit_turn_item_completed_in_commit(&turn, item)
                    .await;
                session.flush_rollout().await.unwrap();
            }
            AbortWindow::CommittingTerminal => {
                let _commit = turn.text_terminal_commit.lock().await;
                if let TurnItem::AgentMessage(message) = &mut item {
                    message.content = vec![AgentMessageContent::Text {
                        text: "kept 中文🙂".into(),
                    }];
                }
                session
                    .send_event_inner(
                        &turn,
                        EventMsg::ItemCompleted(ItemCompletedEvent {
                            thread_id: session.thread_id,
                            turn_id: turn.sub_id.clone(),
                            item,
                            started_at_ms: None,
                            completed_at_ms: 1,
                        }),
                    )
                    .await;
                self.ready.notify_one();
                // Hold the real commit guard after append, before its flush.
                self.release.notified().await;
                session.flush_rollout().await.unwrap();
            }
        }
        // Deliberately ignore cooperative cancellation to exercise handle.abort.
        std::future::pending().await
    }
}

#[tokio::test]
async fn forced_task_abort_joins_worker_and_preserves_one_terminal() {
    for window in [
        AbortWindow::OpenItem,
        AbortWindow::CommittingTerminal,
        AbortWindow::RawRecordedBeforeStart,
        AbortWindow::AuthoritativeAfterStream,
        AbortWindow::RawCommitInFlight,
    ] {
        let (mut session, turn, rx) =
            crate::session::tests::make_session_and_context_with_rx().await;
        let path =
            crate::session::tests::attach_thread_persistence(Arc::get_mut(&mut session).unwrap())
                .await;
        let ready = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        session
            .spawn_task(
                Arc::clone(&turn),
                vec![],
                StuckTextTask {
                    window,
                    ready: Arc::clone(&ready),
                    release: Arc::clone(&release),
                    dropped: Arc::clone(&dropped),
                },
            )
            .await;
        ready.notified().await;
        let cancellation = session
            .active_turn
            .lock()
            .await
            .as_ref()
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .cancellation_token
            .clone();
        let mut raw_guard = if matches!(window, AbortWindow::RawCommitInFlight) {
            Some(session.state.lock().await)
        } else {
            None
        };
        if matches!(window, AbortWindow::RawCommitInFlight) {
            release.notify_one();
        }
        let abort_session = Arc::clone(&session);
        let abort = tokio::spawn(async move {
            abort_session
                .abort_all_tasks(TurnAbortReason::Interrupted)
                .await;
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), cancellation.cancelled())
            .await
            .unwrap();
        if matches!(
            window,
            AbortWindow::CommittingTerminal | AbortWindow::RawCommitInFlight
        ) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            assert!(!dropped.load(Ordering::SeqCst));
            assert!(!abort.is_finished());
            if matches!(window, AbortWindow::RawCommitInFlight) {
                drop(raw_guard.take());
            } else {
                release.notify_one();
            }
        }
        abort.await.unwrap();
        assert!(
            dropped.load(Ordering::SeqCst),
            "worker must be joined before terminal repair"
        );
        let mut terminals = vec![];
        while let Ok(event) = rx.try_recv() {
            match event.msg {
                EventMsg::ItemCompleted(event) if event.item.id() == "forced-item" => {
                    terminals.push(event.item)
                }
                EventMsg::TurnAborted(_) => assert_eq!(terminals.len(), 1),
                _ => {}
            }
        }
        assert_eq!(terminals.len(), 1);
        let authoritative = matches!(
            window,
            AbortWindow::RawRecordedBeforeStart
                | AbortWindow::AuthoritativeAfterStream
                | AbortWindow::RawCommitInFlight
        );
        let expected = if authoritative {
            "kept authoritative 中文🙂"
        } else {
            "kept 中文🙂"
        };
        assert_eq!(
            serde_json::to_value(&terminals[0]).unwrap()["content"][0]["text"],
            expected
        );
        if authoritative {
            let prompt = session.clone_history().await.for_prompt(&[]);
            assert_eq!(
                prompt
                    .iter()
                    .filter(|item| item.id().is_some_and(|id| id.as_str() == "forced-item"))
                    .count(),
                1
            );
        }
        let persisted = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(codex_rollout::parse_rollout_line)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let completed = persisted
            .iter()
            .filter_map(|line| match &line.item {
                codex_rollout::RolloutItem::EventMsg(EventMsg::ItemCompleted(event))
                    if event.item.id() == "forced-item" =>
                {
                    Some(&event.item)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if authoritative {
            assert_eq!(
                persisted
                    .iter()
                    .filter(|line| matches!(&line.item,
                codex_rollout::RolloutItem::ResponseItem(envelope)
                    if envelope.item.id().is_some_and(|id| id.as_str() == "forced-item")))
                    .count(),
                1
            );
        }
        assert_eq!(completed.len(), 1);
        assert_eq!(
            serde_json::to_value(completed[0]).unwrap(),
            serde_json::to_value(&terminals[0]).unwrap()
        );
    }
}

/// r56 port note: 0.163 returned early (lifecycle only, no TurnAborted) when the
/// task token was already cancelled. r52.4 removed that shortcut so the abort
/// path still joins the worker, repairs open text items and emits one terminal.
#[tokio::test]
async fn already_cancelled_task_abort_still_repairs_and_emits_one_turn_aborted() {
    let (session, turn, rx) = crate::session::tests::make_session_and_context_with_rx().await;
    let ready = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    session
        .spawn_task(
            Arc::clone(&turn),
            vec![],
            StuckTextTask {
                window: AbortWindow::OpenItem,
                ready: Arc::clone(&ready),
                release: Arc::new(Notify::new()),
                dropped: Arc::clone(&dropped),
            },
        )
        .await;
    ready.notified().await;
    session
        .active_turn
        .lock()
        .await
        .as_ref()
        .unwrap()
        .task
        .as_ref()
        .unwrap()
        .cancellation_token
        .cancel();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.abort_all_tasks(TurnAbortReason::Interrupted),
    )
    .await
    .unwrap();
    assert!(
        dropped.load(Ordering::SeqCst),
        "worker must be joined even when its token was already cancelled"
    );
    let mut terminals = 0;
    let mut aborted = 0;
    while let Ok(event) = rx.try_recv() {
        match event.msg {
            EventMsg::ItemCompleted(event) if event.item.id() == "forced-item" => {
                assert_eq!(aborted, 0, "terminal must precede TurnAborted");
                terminals += 1;
            }
            EventMsg::TurnAborted(_) => aborted += 1,
            _ => {}
        }
    }
    assert_eq!((terminals, aborted), (1, 1));
}
