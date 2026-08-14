//! End-to-end IPC test: spawns the real `gramitd` binary, talks to it over the real
//! local socket, and checks the protocol from the outside — the same way the CLI will
//! in Module 3.

use std::io::Write as _;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use gramit_core::ipc::{self, Request, Response};
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, Name};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Kills the daemon even when a test fails partway through.
struct DaemonGuard {
    child: Child,
    socket: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn socket_name(path: &std::path::Path) -> Name<'static> {
    path.to_path_buf().into_os_string().to_fs_name::<GenericFilePath>().unwrap()
}

/// Starts a daemon in an isolated temp dir, pointed at a backend port nothing listens
/// on, so backend calls fail fast and deterministically.
async fn start_daemon() -> DaemonGuard {
    let dir = tempfile::tempdir().expect("temp dir");
    let socket = dir.path().join("gramit.sock");
    let config_path = dir.path().join("config.toml");

    let mut config = std::fs::File::create(&config_path).expect("config file");
    writeln!(
        config,
        "hotkey = \"Ctrl+Alt+F\"\n\
         backend_url = \"http://127.0.0.1:1\"\n\
         mode = \"grammar\"\n\
         notifications = false\n\
         max_chars = 20\n\
         request_timeout_ms = 500"
    )
    .expect("write config");
    drop(config);

    let child = Command::new(env!("CARGO_BIN_EXE_gramitd"))
        .env("GRAMIT_SOCKET", &socket)
        .env("GRAMIT_CONFIG", &config_path)
        .env("GRAMIT_LOG", dir.path().join("gramitd.log"))
        .spawn()
        .expect("spawn gramitd");

    let guard = DaemonGuard { child, socket: socket.clone(), _dir: dir };

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if Stream::connect(socket_name(&socket)).await.is_ok() {
            return guard;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("gramitd did not start listening on {}", socket.display());
}

/// Sends requests on one connection and returns the responses in order.
async fn round_trip(socket: &std::path::Path, requests: &[Request]) -> Vec<Response> {
    let stream = Stream::connect(socket_name(socket)).await.expect("connect");
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

async fn send_raw(socket: &std::path::Path, raw: &str) -> Response {
    let stream = Stream::connect(socket_name(socket)).await.expect("connect");
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
    let responses = round_trip(&daemon.socket, &[Request::Ping]).await;

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
    let responses = round_trip(&daemon.socket, &[Request::Ping, Request::Ping, Request::Status]).await;

    assert_eq!(responses.len(), 3);
    assert!(matches!(responses[0], Response::Pong { .. }));
    assert!(matches!(responses[1], Response::Pong { .. }));
    assert!(matches!(responses[2], Response::Status(_)));
}

#[tokio::test]
async fn status_reports_config_and_an_unreachable_backend() {
    let daemon = start_daemon().await;
    let responses = round_trip(&daemon.socket, &[Request::Status]).await;

    match &responses[0] {
        Response::Status(report) => {
            assert_eq!(report.hotkey, "Ctrl+Alt+F");
            assert_eq!(report.backend_url, "http://127.0.0.1:1");
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
    let responses = round_trip(&daemon.socket, &[request]).await;

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
    let responses = round_trip(&daemon.socket, &[request]).await;

    match &responses[0] {
        Response::Error { code, .. } => assert_eq!(code, "TOO_LONG"),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_input_gets_an_error_not_a_dropped_connection() {
    let daemon = start_daemon().await;

    match send_raw(&daemon.socket, "this is not json\n").await {
        Response::Error { code, .. } => assert_eq!(code, "BAD_REQUEST"),
        other => panic!("expected an error, got {other:?}"),
    }

    // The daemon must still be healthy afterwards.
    let responses = round_trip(&daemon.socket, &[Request::Ping]).await;
    assert!(matches!(responses[0], Response::Pong { .. }));
}

#[tokio::test]
async fn a_second_daemon_refuses_to_start_on_a_live_socket() {
    let daemon = start_daemon().await;

    let output = Command::new(env!("CARGO_BIN_EXE_gramitd"))
        .env("GRAMIT_SOCKET", &daemon.socket)
        .env("GRAMIT_CONFIG", daemon.socket.with_file_name("config.toml"))
        .env("GRAMIT_LOG", daemon.socket.with_file_name("second.log"))
        .output()
        .expect("run second gramitd");

    assert!(!output.status.success(), "the second daemon should have refused to start");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already running"), "unexpected stderr: {stderr}");
}

#[tokio::test]
async fn a_stale_socket_file_does_not_block_startup() {
    let dir = tempfile::tempdir().expect("temp dir");
    let socket = dir.path().join("stale.sock");
    // A leftover file where the socket belongs, with nothing listening on it.
    std::fs::write(&socket, b"leftover").expect("write stale socket");

    let child = Command::new(env!("CARGO_BIN_EXE_gramitd"))
        .env("GRAMIT_SOCKET", &socket)
        .env("GRAMIT_CONFIG", dir.path().join("missing.toml"))
        .env("GRAMIT_LOG", dir.path().join("gramitd.log"))
        .spawn()
        .expect("spawn gramitd");
    let guard = DaemonGuard { child, socket: socket.clone(), _dir: dir };

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut connected = false;
    while Instant::now() < deadline {
        if Stream::connect(socket_name(&socket)).await.is_ok() {
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
    let responses = round_trip(&daemon.socket, &[Request::Shutdown]).await;
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
    assert!(!daemon.socket.exists(), "the socket file should be cleaned up on shutdown");
}
