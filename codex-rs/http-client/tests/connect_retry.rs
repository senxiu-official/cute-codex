#![allow(clippy::expect_used)]
//! Real loopback TCP/TLS failures, with no supplier or account credentials.

use codex_http_client::HttpTransport;
use codex_http_client::Request;
use codex_http_client::ReqwestTransport;
use codex_http_client::TransportError;
use futures::StreamExt;
use http::Method;
use rcgen::CertificateParams;
use rcgen::KeyPair;
use rustls_pki_types::PrivateKeyDer;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy)]
enum Scenario {
    TlsResetThenSuccess,
    TlsInvalidThenSuccess,
    TlsExhausted,
    Status(u16),
    BodyDrop,
    SendAmbiguous,
    Redirect,
}

struct Upstream {
    url: String,
    transport: ReqwestTransport,
    connections: Arc<AtomicUsize>,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl Upstream {
    fn start(scenario: Scenario) -> Self {
        Self::start_with_budget(scenario, /*max_retries*/ 2)
    }

    fn start_with_budget(scenario: Scenario, max_retries: u64) -> Self {
        codex_utils_rustls_provider::ensure_rustls_crypto_provider();
        let listener = TcpListener::bind("127.0.0.1:0").expect("loopback listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let address = listener.local_addr().expect("loopback address");
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let redirected = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&redirected);
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                flag.store(true, Ordering::Relaxed);
                reqwest::redirect::Policy::default().redirect(attempt)
            }));
        let tls = matches!(
            scenario,
            Scenario::TlsResetThenSuccess
                | Scenario::TlsInvalidThenSuccess
                | Scenario::TlsExhausted
        );
        let config = if tls {
            let key = KeyPair::generate().expect("test TLS key");
            let cert = CertificateParams::new(vec!["127.0.0.1".to_owned()])
                .expect("test certificate parameters")
                .self_signed(&key)
                .expect("test certificate");
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_der(cert.der()).expect("test trust anchor"),
            );
            Some(Arc::new(
                rustls::ServerConfig::builder()
                    .with_no_client_auth()
                    .with_single_cert(vec![cert.der().clone()], PrivateKeyDer::from(key))
                    .expect("test TLS server configuration"),
            ))
        } else {
            None
        };
        let transport = ReqwestTransport::new(builder.build().expect("test HTTP client"))
            .with_connect_retries(redirected, max_retries);
        let count = Arc::clone(&connections);
        let http_count = Arc::clone(&requests);
        let stopping = Arc::clone(&stop);
        let server = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("accept fixture connection: {e}"),
                };
                // Required on Windows even though the listener is nonblocking.
                socket
                    .set_nonblocking(false)
                    .expect("blocking accepted socket");
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .expect("write timeout");
                let number = count.fetch_add(1, Ordering::Relaxed) + 1;
                let fail_tls = matches!(scenario, Scenario::TlsExhausted)
                    || number == 1
                        && matches!(
                            scenario,
                            Scenario::TlsResetThenSuccess | Scenario::TlsInvalidThenSuccess
                        );
                if fail_tls {
                    let mut hello = [0; 5];
                    let _ = socket.read(&mut hello);
                    if matches!(scenario, Scenario::TlsInvalidThenSuccess) {
                        let _ = socket.write_all(b"not a TLS record");
                    }
                    continue;
                }
                if let Some(config) = &config {
                    let session =
                        rustls::ServerConnection::new(Arc::clone(config)).expect("TLS session");
                    let mut stream = rustls::StreamOwned::new(session, socket);
                    serve(&mut stream, scenario, &http_count);
                } else {
                    serve(&mut socket, scenario, &http_count);
                }
            }
        });
        Self {
            url: format!(
                "{}://{address}/responses",
                if tls { "https" } else { "http" }
            ),
            transport,
            connections,
            requests,
            stop,
            server: Some(server),
        }
    }

    fn request(&self) -> Request {
        Request::new(Method::POST, self.url.clone()).with_raw_body("synthetic model request")
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(server) = self.server.take() {
            server.join().expect("fixture server stopped");
        }
    }
}

fn serve(stream: &mut (impl Read + Write), scenario: Scenario, count: &AtomicUsize) {
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        if stream.read_exact(&mut byte).is_err() {
            return;
        }
        headers.push(byte[0]);
        assert!(headers.len() < 16384);
    }
    let headers = String::from_utf8(headers).expect("fixture headers");
    let size = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .unwrap_or(0);
    let mut body = vec![0; size];
    if stream.read_exact(&mut body).is_err() {
        return;
    }
    count.fetch_add(1, Ordering::Relaxed);
    if matches!(scenario, Scenario::SendAmbiguous) {
        return;
    }
    let response = match scenario {
        Scenario::BodyDrop => {
            "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\npartial".to_owned()
        }
        Scenario::Redirect => {
            let unused = TcpListener::bind("127.0.0.1:0").expect("unused redirect port");
            let address = unused.local_addr().expect("unused redirect address");
            drop(unused);
            format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{address}/redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
        }
        Scenario::Status(status) => {
            format!("HTTP/1.1 {status} Error\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno")
        }
        _ => "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned(),
    };
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

#[tokio::test]
async fn connection_reset_before_tls_then_success() {
    let upstream = Upstream::start(Scenario::TlsResetThenSuccess);
    let response = upstream
        .transport
        .execute(upstream.request())
        .await
        .expect("retry succeeded");
    assert_eq!(response.body.as_ref(), b"ok");
    assert_eq!(upstream.connections.load(Ordering::Relaxed), 2);
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn invalid_tls_handshake_then_success() {
    let upstream = Upstream::start(Scenario::TlsInvalidThenSuccess);
    let response = upstream
        .transport
        .execute(upstream.request())
        .await
        .expect("retry succeeded");
    assert_eq!(response.body.as_ref(), b"ok");
    assert_eq!(upstream.connections.load(Ordering::Relaxed), 2);
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn connection_retry_exhaustion_returns_last_error_after_three_connections() {
    let upstream = Upstream::start_with_budget(Scenario::TlsExhausted, /*max_retries*/ 4);
    let error = upstream
        .transport
        .stream(upstream.request())
        .await
        .err()
        .expect("connection failed");
    assert!(matches!(error, TransportError::Connection(_)));
    assert_eq!(upstream.connections.load(Ordering::Relaxed), 3);
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn an_explicit_zero_request_retry_budget_is_respected() {
    let upstream = Upstream::start_with_budget(Scenario::TlsExhausted, /*max_retries*/ 0);
    let error = upstream
        .transport
        .stream(upstream.request())
        .await
        .err()
        .expect("connection failed");
    assert!(matches!(error, TransportError::Connection(_)));
    assert_eq!(upstream.connections.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn statuses_including_auth_quota_and_bad_request_are_not_retried() {
    for status in [400, 401, 429, 500, 529] {
        let upstream = Upstream::start(Scenario::Status(status));
        let error = upstream
            .transport
            .stream(upstream.request())
            .await
            .err()
            .expect("HTTP error");
        assert!(
            matches!(error, TransportError::Http { status: actual, .. } if actual.as_u16() == status)
        );
        assert_eq!(upstream.connections.load(Ordering::Relaxed), 1);
        assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn stream_drop_after_response_bytes_is_not_replayed() {
    let upstream = Upstream::start(Scenario::BodyDrop);
    let mut response = upstream
        .transport
        .stream(upstream.request())
        .await
        .expect("response headers");
    let mut failed = false;
    while let Some(bytes) = response.bytes.next().await {
        if bytes.is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
    assert_eq!(upstream.connections.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn ambiguous_send_failure_is_not_retried() {
    let upstream = Upstream::start(Scenario::SendAmbiguous);
    let error = upstream
        .transport
        .stream(upstream.request())
        .await
        .err()
        .expect("ambiguous send failed");
    assert!(matches!(
        error,
        TransportError::Network(_) | TransportError::Timeout
    ));
    assert_eq!(upstream.connections.load(Ordering::Relaxed), 1);
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn connection_failure_after_redirect_does_not_replay_the_original_post() {
    let upstream = Upstream::start(Scenario::Redirect);
    let result = tokio::time::timeout(
        // Windows may spend over a second rejecting the redirected connection.
        // The request count below detects replay without an OS timing assumption.
        Duration::from_secs(8),
        upstream.transport.stream(upstream.request()),
    )
    .await;
    assert!(
        result
            .expect("redirected connection failure must finish")
            .is_err()
    );
    assert_eq!(upstream.requests.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn cancellation_interrupts_backoff_without_another_connection() {
    let upstream = Upstream::start(Scenario::TlsExhausted);
    let transport = upstream.transport.clone();
    let request = upstream.request();
    let task = tokio::spawn(async move { transport.stream(request).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while upstream.connections.load(Ordering::Relaxed) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("first connection started");
    tokio::time::sleep(Duration::from_millis(100)).await;
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(upstream.connections.load(Ordering::Relaxed), 1);
}
