//! Portable real-process checks for definite idle interruption replies.
use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnInterruptParams;
use codex_app_server_protocol::TurnInterruptResponse;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::TurnStartedNotification;
use codex_app_server_protocol::TurnStatus;
use codex_app_server_protocol::UserInput;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

async fn fixture() -> Result<(TempDir, MockServer, TestAppServer, String)> {
    let home = TempDir::new()?;
    let server = MockServer::start().await;
    MockResponsesConfig::new(&server.uri())
        .with_sandbox_mode("danger-full-access")
        .with_root_config("analytics.enabled = false\nfeatures.instant_interrupt = false")
        .write(home.path())?;
    let mut app = TestAppServer::builder()
        .without_auto_env()
        .without_managed_config()
        .with_codex_home(home.path())
        .build_initialized()
        .await?;
    let ThreadStartResponse { thread, .. } = app
        .request(|request_id| ClientRequest::ThreadStart {
            request_id,
            params: ThreadStartParams {
                cwd: Some(home.path().to_string_lossy().into_owned()),
                ..Default::default()
            },
        })
        .await?;
    Ok((home, server, app, thread.id))
}

async fn assert_definite_idle(app: &mut TestAppServer, thread: &str, turn: &str) -> Result<()> {
    let id = app
        .send_turn_interrupt_request(TurnInterruptParams {
            thread_id: thread.into(),
            turn_id: turn.into(),
        })
        .await?;
    let response: JSONRPCError = timeout(
        REPLY_TIMEOUT,
        app.read_stream_until_error_message(RequestId::Integer(id)),
    )
    .await??;
    assert_eq!(response.error.code, -32600);
    assert_eq!(response.error.message, "no active turn to interrupt");
    Ok(())
}

#[tokio::test]
async fn named_interrupt_on_fresh_idle_thread_has_a_definite_reply() -> Result<()> {
    let (_home, _server, mut app, thread) = fixture().await?;
    assert_definite_idle(&mut app, &thread, "no-such-turn").await
}

#[tokio::test]
async fn named_interrupt_after_owner_stop_has_a_definite_reply() -> Result<()> {
    let (_home, server, mut app, thread) = fixture().await?;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"held\"}}\n\n")
            .set_delay(Duration::from_secs(60)))
        .mount(&server).await;
    let TurnStartResponse { turn } = app
        .request(|request_id| ClientRequest::TurnStart {
            request_id,
            params: TurnStartParams {
                thread_id: thread.clone(),
                input: vec![UserInput::Text {
                    text: "wait for Stop".into(),
                    text_elements: Vec::new(),
                }],
                ..Default::default()
            },
        })
        .await?;
    let _: TurnStartedNotification = timeout(
        Duration::from_secs(10),
        app.read_notification("turn/started"),
    )
    .await??;
    let _: TurnInterruptResponse = timeout(
        REPLY_TIMEOUT,
        app.request(|request_id| ClientRequest::TurnInterrupt {
            request_id,
            params: TurnInterruptParams {
                thread_id: thread.clone(),
                turn_id: turn.id.clone(),
            },
        }),
    )
    .await??;
    let completed: TurnCompletedNotification =
        timeout(REPLY_TIMEOUT, app.read_notification("turn/completed")).await??;
    assert_eq!(completed.turn.status, TurnStatus::Interrupted);
    assert_definite_idle(&mut app, &thread, &turn.id).await
}
