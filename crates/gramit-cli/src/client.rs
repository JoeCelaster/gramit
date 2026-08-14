//! Talking to the daemon over the local socket.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use gramit_core::ipc::{self, Request, Response};
use gramit_core::paths::{self, Endpoint};
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, Name};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn to_name(endpoint: &Endpoint) -> Result<Name<'static>> {
    match endpoint {
        Endpoint::Path(path) => path
            .clone()
            .into_os_string()
            .to_fs_name::<GenericFilePath>()
            .with_context(|| format!("invalid socket path {}", path.display())),
        Endpoint::Namespaced(name) => name
            .clone()
            .to_ns_name::<GenericNamespaced>()
            .with_context(|| format!("invalid socket name {name}")),
    }
}

/// Sends one request and returns the daemon's reply.
pub async fn request(request: Request) -> Result<Response> {
    let endpoint = paths::endpoint();
    let stream = Stream::connect(to_name(&endpoint)?).await.map_err(|err| {
        anyhow!("could not reach the gramit daemon at {endpoint} ({err}).\nStart it with: gramit start")
    })?;

    let (recv, mut send) = stream.split();
    let mut reader = BufReader::new(recv);

    send.write_all(ipc::encode(&request)?.as_bytes()).await.context("could not send the request")?;
    send.flush().await.context("could not flush the request")?;

    let mut line = String::new();
    reader.read_line(&mut line).await.context("could not read the daemon's reply")?;

    if line.trim().is_empty() {
        return Err(anyhow!("the daemon closed the connection without replying"));
    }

    ipc::decode_response(line.trim()).context("could not parse the daemon's reply")
}

/// Whether a daemon is currently listening.
pub async fn is_running() -> bool {
    matches!(request(Request::Ping).await, Ok(Response::Pong { .. }))
}

/// Waits for the daemon to start answering, up to `timeout`.
pub async fn wait_until_ready(timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if is_running().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits for the daemon to stop answering, up to `timeout`.
pub async fn wait_until_stopped(timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !is_running().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
