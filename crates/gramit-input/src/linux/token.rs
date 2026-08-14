//! Persistence for the RemoteDesktop restore token.
//!
//! Without this the user is asked to allow remote control on every daemon start.
//! With it, the portal recognises the session and stays silent.

use std::path::PathBuf;

use tracing::{debug, warn};

fn token_path() -> Option<PathBuf> {
    gramit_core::paths::state_dir().ok().map(|dir| dir.join("remote-desktop.token"))
}

pub fn load() -> Option<String> {
    let path = token_path()?;
    match std::fs::read_to_string(&path) {
        Ok(token) => {
            let token = token.trim().to_string();
            if token.is_empty() {
                None
            } else {
                debug!(path = %path.display(), "reusing the saved portal restore token");
                Some(token)
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            warn!(path = %path.display(), %err, "could not read the portal restore token");
            None
        }
    }
}

pub fn save(token: &str) {
    let Some(path) = token_path() else { return };

    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            warn!(path = %parent.display(), %err, "could not create the state directory");
            return;
        }
    }

    match std::fs::write(&path, token) {
        // The token authorises input injection, so keep it out of other users' reach.
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
            debug!(path = %path.display(), "saved the portal restore token");
        }
        Err(err) => warn!(path = %path.display(), %err, "could not save the portal restore token"),
    }
}

/// Drops a token the portal has rejected, so the next start asks the user afresh.
pub fn clear() {
    let Some(path) = token_path() else { return };
    match std::fs::remove_file(&path) {
        Ok(()) => debug!("cleared the stale portal restore token"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => warn!(%err, "could not clear the portal restore token"),
    }
}
