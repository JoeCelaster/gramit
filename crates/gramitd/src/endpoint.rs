//! Binding and connecting to the local socket, with the platform differences
//! (filesystem socket vs. named pipe) contained here.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use gramit_core::paths::Endpoint;
use interprocess::local_socket::tokio::{prelude::*, Listener, Stream};
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Name};
use tracing::{debug, warn};

pub fn to_name(endpoint: &Endpoint) -> Result<Name<'static>> {
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

/// True if something is already accepting connections at `endpoint`.
///
/// Used to tell a live daemon (refuse to start) from a socket file left behind by a
/// crash (delete it and carry on).
pub async fn is_listening(endpoint: &Endpoint) -> bool {
    let Ok(name) = to_name(endpoint) else {
        return false;
    };
    matches!(
        tokio::time::timeout(Duration::from_millis(500), Stream::connect(name)).await,
        Ok(Ok(_))
    )
}

pub async fn bind(endpoint: &Endpoint) -> Result<Listener> {
    if is_listening(endpoint).await {
        return Err(anyhow!("gramitd is already running on {endpoint}"));
    }

    // Nothing answered, so any socket file here is a leftover from a crash.
    if let Endpoint::Path(path) = endpoint {
        if path.exists() {
            warn!(path = %path.display(), "removing stale socket");
            std::fs::remove_file(path)
                .with_context(|| format!("could not remove stale socket {}", path.display()))?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
    }

    let listener = ListenerOptions::new()
        .name(to_name(endpoint)?)
        .create_tokio()
        .with_context(|| format!("could not listen on {endpoint}"))?;

    restrict_permissions(endpoint)?;
    debug!(%endpoint, "listening");
    Ok(listener)
}

/// The socket carries the user's text, so it must not be world-accessible. Under
/// $XDG_RUNTIME_DIR the 0700 parent already covers this; the /tmp fallback does not.
#[cfg(unix)]
fn restrict_permissions(endpoint: &Endpoint) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    if let Endpoint::Path(path) = endpoint {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("could not chmod {}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_endpoint: &Endpoint) -> Result<()> {
    // Windows named pipes created by interprocess default to the creating user.
    Ok(())
}

/// Best-effort removal of the socket file on shutdown.
pub fn cleanup(endpoint: &Endpoint) {
    if let Endpoint::Path(path) = endpoint {
        if let Err(err) = std::fs::remove_file(path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %path.display(), %err, "could not remove socket");
            }
        }
    }
}
