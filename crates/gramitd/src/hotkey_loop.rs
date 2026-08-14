use std::sync::Arc;

use gramit_input::HotkeyRegistration;
use tracing::{info, warn};

use crate::fixloop::FixOutcome;
use crate::selection;
use crate::shutdown::Shutdown;
use crate::state::DaemonState;

/// Consumes hotkey presses and runs a fix for each one.
///
/// Owns the registration for its whole life: dropping it would close the portal
/// session and silently unbind the shortcut.
pub async fn run(mut registration: HotkeyRegistration, state: Arc<DaemonState>, shutdown: Shutdown) {
    info!(hotkey = %registration.description, "hotkey loop started");

    loop {
        let event = tokio::select! {
            biased;
            _ = shutdown.wait() => break,
            event = registration.events.recv() => event,
        };

        let Some(event) = event else {
            warn!("the hotkey channel closed; the shortcut is no longer live");
            break;
        };

        info!(id = %event.id, "hotkey pressed");

        // Presses arrive one at a time here, and `selection::run` refuses overlapping
        // fixes, so a leaned-on key can't stack up pastes. It also raises the desktop
        // notification, so the log below is just for `gramit logs`.
        match selection::run(&state).await {
            FixOutcome::Replaced { changes, .. } => info!(changes, "fixed the selection"),
            FixOutcome::AlreadyCorrect { .. } => info!("selection was already correct"),
            FixOutcome::NoSelection => info!("nothing was selected"),
            FixOutcome::Failed { code, message, .. } => warn!(%code, %message, "the fix failed"),
        }
    }

    info!("hotkey loop stopped");
}
