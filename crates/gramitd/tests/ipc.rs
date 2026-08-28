//! End-to-end IPC test: spawns the real `gramitd` binary, talks to it over the real
//! local socket, and checks the protocol from the outside — the same way the CLI will
//! in Module 3.

use std::io::Write as _;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use gramit_core::ipc::{self, Request, Response};
use gramit_core::paths::{self, Endpoint};
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::Name;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Kills the daemon even when a test fails partway through.
struct DaemonGuard {
    child: Child,
    /// Where the daemon is listening, in whatever form this platform uses: a socket
    /// file on Unix, a named pipe on Windows.
    endpoint: Endpoint,
    /// The temp directory holding the config and log, which is not where the socket
    /// lives on Windows.
    dir: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The same conversion the daemon and the CLI use. Building the name here by hand is
/// what made every one of these tests fail on Windows: they asked for a filesystem
/// socket while the daemon was opening a named pipe.
fn socket_name(endpoint: &Endpoint) -> Name<'static> {
    paths::to_name(endpoint).expect("socket name")
}

/// A unique label per daemon, so tests running in parallel never share a pipe name.
fn next_label(prefix: &str) -> String {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    format!("{prefix}-{}.sock", COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// Starts a daemon in an isolated temp dir, pointed at a backend port nothing listens
/// on, so backend calls fail fast and deterministically.
async fn start_daemon() -> DaemonGuard {
    start_daemon_with("backend_url = \"http://127.0.0.1:1\"\n").await
}

/// The same daemon with `extra` appended to its config, so a test can vary one
/// setting — notably by leaving `backend_url` out entirely.
async fn start_daemon_with(extra: &str) -> DaemonGuard {
    let dir = tempfile::tempdir().expect("temp dir");
    let endpoint = Endpoint::for_test(dir.path(), &next_label("gramit"));
    let config_path = dir.path().join("config.toml");

    let mut config = std::fs::File::create(&config_path).expect("config file");
    writeln!(
        config,
        "hotkey = \"Ctrl+Alt+F\"\n\
         mode = \"code\"\n\
         notifications = false\n\
         max_chars = 20\n\
         request_timeout_ms = 500\n\
         {extra}"
    )
    .expect("write config");
    drop(config);

    let child = Command::new(env!("CARGO_BIN_EXE_gramitd"))
        .env("GRAMIT_SOCKET", endpoint.as_env_value())
        .env("GRAMIT_CONFIG", &config_path)
        // The daemon honours this at run time, so a developer who has it exported
        // would otherwise silently change what these tests are asserting about.
        .env_remove("GRAMIT_BACKEND_URL")
        .env("GRAMIT_LOG", dir.path().join("gramitd.log"))
        .spawn()
        .expect("spawn gramitd");

    let guard = DaemonGuard {
        child,
        endpoint: endpoint.clone(),
        dir: dir.path().to_path_buf(),
        _dir: dir,
    };

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if Stream::connect(socket_name(&endpoint)).await.is_ok() {
            return guard;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("gramitd did not start listening on {endpoint}");
}

/// Sends requests on one connection and returns the responses in order.
async fn round_trip(endpoint: &Endpoint, requests: &[Request]) -> Vec<Response> {
    let stream = Stream::connect(socket_name(endpoint)).await.expect("connect");
    let (recv, mut send) = stream.split();
    let mut reader = BufReader::new(recv);
    let mut responses = Vec::new();

    for request in requests {
        send.write_all(ipc::encode(request).unwrap().as_bytes()).await.expect("write");
        send.flush().await.expect("flush");

        let mut line = String::new();
        reader.read_line(&mut line).await.expect("read");
        responses.push(ipc::decode_response(line.trim()).expect("decode"));
    }

    responses
}

async fn send_raw(endpoint: &Endpoint, raw: &str) -> Response {
    let stream = Stream::connect(socket_name(endpoint)).await.expect("connect");
    let (recv, mut send) = stream.split();
    let mut reader = BufReader::new(recv);

    send.write_all(raw.as_bytes()).await.expect("write");
    send.flush().await.expect("flush");

    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read");
    ipc::decode_response(line.trim()).expect("decode")
}

#[tokio::test]
async fn ping_returns_the_daemon_pid_and_version() {
    let daemon = start_daemon().await;
    let responses = round_trip(&daemon.endpoint, &[Request::Ping]).await;

    match &responses[0] {
        Response::Pong { version, pid, .. } => {
            assert_eq!(version, gramit_core::VERSION);
            assert_eq!(*pid, daemon.child.id());
        }
        other => panic!("expected pong, got {other:?}"),
    }
}

#[tokio::test]
async fn one_connection_carries_several_requests() {
    let daemon = start_daemon().await;
    let responses = round_trip(&daemon.endpoint, &[Request::Ping, Request::Ping, Request::Status]).await;

    assert_eq!(responses.len(), 3);
    assert!(matches!(responses[0], Response::Pong { .. }));
    assert!(matches!(responses[1], Response::Pong { .. }));
    assert!(matches!(responses[2], Response::Status(_)));
}

/// A brand-new install: no backend has been named yet. The daemon must still come
/// up and serve IPC, and it must refuse to correct anything.
#[tokio::test]
async fn an_unconfigured_daemon_still_starts_and_refuses_to_fix() {
    let daemon = start_daemon_with("").await;
    let responses = round_trip(
        &daemon.endpoint,
        &[Request::Status, Request::Fix { text: "he go".into(), mode: Default::default() }],
    )
    .await;

    match &responses[0] {
        Response::Status(report) => {
            assert_eq!(report.backend_url, None, "nothing may be assumed about the backend");
            assert!(!report.backend_reachable);
        }
        other => panic!("expected status, got {other:?}"),
    }

    match &responses[1] {
        Response::Error { code, message, .. } => {
            assert_eq!(code, "NO_BACKEND");
            assert!(message.contains("gramit setup"), "the error must say how to fix it: {message}");
        }
        other => panic!("expected an error, got {other:?}"),
    }
}

#[tokio::test]
async fn status_reports_config_and_an_unreachable_backend() {
    let daemon = start_daemon().await;
    let responses = round_trip(&daemon.endpoint, &[Request::Status]).await;

    match &responses[0] {
        Response::Status(report) => {
            assert_eq!(report.hotkey, "Ctrl+Alt+F");
            assert_eq!(report.backend_url.as_deref(), Some("http://127.0.0.1:1"));
            assert!(!report.backend_reachable);
            assert!(!report.notifications);
            assert_eq!(report.fixes_total, 0);
        }
        other => panic!("expected status, got {other:?}"),
    }
}

#[tokio::test]
async fn fix_surfaces_the_backend_error_code() {
    let daemon = start_daemon().await;
    let request = Request::Fix { text: "he go".into(), mode: Default::default() };
    let responses = round_trip(&daemon.endpoint, &[request]).await;

    match &responses[0] {
        Response::Error { code, retryable, .. } => {
            assert_eq!(code, "BACKEND_UNREACHABLE");
            assert!(retryable);
        }
        other => panic!("expected an error, got {other:?}"),
    }
}

#[tokio::test]
async fn fix_enforces_the_configured_character_limit() {
    let daemon = start_daemon().await;
    // The test config sets max_chars = 20.
    let request = Request::Fix { text: "a".repeat(21), mode: Default::default() };
    let responses = round_trip(&daemon.endpoint, &[request]).await;

    match &responses[0] {
        Response::Error { code, .. } => assert_eq!(code, "TOO_LONG"),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_input_gets_an_error_not_a_dropped_connection() {
    let daemon = start_daemon().await;

    match send_raw(&daemon.endpoint, "this is not json\n").await {
        Response::Error { code, .. } => assert_eq!(code, "BAD_REQUEST"),
        other => panic!("expected an error, got {other:?}"),
    }

    // The daemon must still be healthy afterwards.
    let responses = round_trip(&daemon.endpoint, &[Request::Ping]).await;
    assert!(matches!(responses[0], Response::Pong { .. }));
}

#[tokio::test]
async fn a_second_daemon_refuses_to_start_on_a_live_socket() {
    let daemon = start_daemon().await;

    let output = Command::new(env!("CARGO_BIN_EXE_gramitd"))
        .env("GRAMIT_SOCKET", daemon.endpoint.as_env_value())
        .env("GRAMIT_CONFIG", daemon.dir.join("config.toml"))
        .env("GRAMIT_LOG", daemon.dir.join("second.log"))
        .output()
        .expect("run second gramitd");

    assert!(!output.status.success(), "the second daemon should have refused to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already running"), "unexpected stderr: {stderr}");
}

/// Unix only: a named pipe lives in its own namespace and leaves no file behind, so
/// there is no such thing as a stale one to clean up on Windows.
#[cfg(unix)]
#[tokio::test]
async fn a_stale_socket_file_does_not_block_startup() {
    let dir = tempfile::tempdir().expect("temp dir");
    let endpoint = Endpoint::for_test(dir.path(), &next_label("stale"));
    let socket = endpoint.socket_file().expect("unix endpoints have a file").to_path_buf();
    // A leftover file where the socket belongs, with nothing listening on it.
    std::fs::write(&socket, b"leftover").expect("write stale socket");

    let child = Command::new(env!("CARGO_BIN_EXE_gramitd"))
        .env("GRAMIT_SOCKET", endpoint.as_env_value())
        .env("GRAMIT_CONFIG", dir.path().join("missing.toml"))
        .env("GRAMIT_LOG", dir.path().join("gramitd.log"))
        .spawn()
        .expect("spawn gramitd");
    let guard = DaemonGuard {
        child,
        endpoint: endpoint.clone(),
        dir: dir.path().to_path_buf(),
        _dir: dir,
    };

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut connected = false;
    while Instant::now() < deadline {
        if Stream::connect(socket_name(&endpoint)).await.is_ok() {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(connected, "gramitd should have replaced the stale socket file");
    drop(guard);
}

#[tokio::test]
async fn shutdown_stops_the_daemon_and_removes_the_socket() {
    let mut daemon = start_daemon().await;
    let responses = round_trip(&daemon.endpoint, &[Request::Shutdown]).await;
    assert_eq!(responses[0], Response::Ok);

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut exited = false;
    while Instant::now() < deadline {
        if matches!(daemon.child.try_wait(), Ok(Some(_))) {
            exited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert!(exited, "gramitd should exit after a shutdown request");
    // Only Unix has a file to clean up; a named pipe disappears with the process that
    // owns it. The exit above is the part that matters on both.
    if let Some(file) = daemon.endpoint.socket_file() {
        assert!(!file.exists(), "the socket file should be cleaned up on shutdown");
    }
}
