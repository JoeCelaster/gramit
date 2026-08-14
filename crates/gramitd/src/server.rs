use std::sync::Arc;

use anyhow::Result;
use gramit_core::ipc::{self, Response, MAX_LINE_BYTES};
use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::tokio::{Listener, Stream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info, warn};

use crate::handler;
use crate::shutdown::Shutdown;
use crate::state::DaemonState;

/// Accepts connections until shutdown is requested.
pub async fn serve(listener: Listener, state: Arc<DaemonState>, shutdown: Shutdown) -> Result<()> {
    info!("accepting connections");

    loop {
        let stream = tokio::select! {
            biased;
            _ = shutdown.wait() => break,
            accepted = listener.accept() => match accepted {
                Ok(stream) => stream,
                Err(err) => {
                    // A single bad connection shouldn't take the daemon down.
                    warn!(%err, "accept failed");
                    continue;
                }
            },
        };

        let state = Arc::clone(&state);
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, state, shutdown).await {
                debug!(%err, "connection ended");
            }
        });
    }

    info!("no longer accepting connections");
    Ok(())
}

async fn handle_connection(stream: Stream, state: Arc<DaemonState>, shutdown: Shutdown) -> Result<()> {
    let (recv, mut send) = stream.split();
    let mut reader = BufReader::new(recv);
    let mut line = String::new();

    loop {
        line.clear();

        let read = tokio::select! {
            biased;
            _ = shutdown.wait() => break,
            read = reader.read_line(&mut line) => read,
        };

        match read {
            Ok(0) => break, // peer hung up
            Ok(_) => {}
            Err(err) => {
                debug!(%err, "read failed");
                break;
            }
        }

        if line.len() > MAX_LINE_BYTES {
            let response = Response::error(
                "REQUEST_TOO_LARGE",
                format!("Request exceeded {MAX_LINE_BYTES} bytes."),
                false,
            );
            write_response(&mut send, &response).await?;
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = match ipc::decode_request(trimmed) {
            Ok(request) => handler::handle(request, &state, &shutdown).await,
            Err(err) => Response::error("BAD_REQUEST", format!("Could not parse request: {err}"), false),
        };

        write_response(&mut send, &response).await?;
    }

    Ok(())
}

async fn write_response<W>(writer: &mut W, response: &Response) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let encoded = match ipc::encode(response) {
        Ok(encoded) => encoded,
        Err(err) => {
            // Serializing our own response type should be impossible to fail; if it
            // somehow does, say so on the wire rather than hanging the client.
            error!(%err, "could not encode response");
            ipc::encode(&Response::error("INTERNAL", "Could not encode the response.", false))?
        }
    };

    writer.write_all(encoded.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}
