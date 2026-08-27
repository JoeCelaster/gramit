use ashpd::desktop::global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut};
use ashpd::desktop::CreateSessionOptions;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::keys::to_portal_trigger;
use crate::hotkey::{HotkeyEvent, HotkeyRegistration, TaskGuard, SHORTCUT_ID};
use crate::InputError;

fn portal_error(context: &str, err: ashpd::Error) -> InputError {
    InputError::Portal(format!("{context}: {err}"))
}

pub async fn register(hotkey: &str) -> Result<HotkeyRegistration, InputError> {
    let trigger = to_portal_trigger(hotkey)?;

    let proxy = GlobalShortcuts::new()
        .await
        .map_err(|err| portal_error("could not reach the GlobalShortcuts portal", err))?;

    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|err| portal_error("could not create a GlobalShortcuts session", err))?;

    // Subscribe before binding: the portal may deliver an activation as soon as the
    // binding exists, and a stream created afterwards would miss it.
    let mut activations = proxy
        .receive_activated()
        .await
        .map_err(|err| portal_error("could not listen for shortcut activations", err))?;

    let shortcut = NewShortcut::new(SHORTCUT_ID, "Fix the current selection")
        .preferred_trigger(trigger.as_str());

    let bound = proxy
        .bind_shortcuts(&session, &[shortcut], None, BindShortcutsOptions::default())
        .await
        .map_err(|err| portal_error("could not bind the shortcut", err))?
        .response()
        .map_err(|err| portal_error("the shortcut binding was refused", err))?;

    // The compositor has the final say, and the user can rebind it in Settings, so
    // report what was actually bound rather than what we asked for.
    let description = bound
        .shortcuts()
        .iter()
        .find(|shortcut| shortcut.id() == SHORTCUT_ID)
        .map(|shortcut| shortcut.trigger_description().to_string())
        .unwrap_or_else(|| {
            warn!("the portal bound no trigger for {SHORTCUT_ID}; the user may need to set one in Settings");
            format!("{trigger} (unconfirmed)")
        });

    info!(hotkey = %description, "global shortcut registered");

    let (events_tx, events_rx) = mpsc::channel(4);

    // This task owns the proxy and session; dropping the registration aborts it,
    // which closes the session and unbinds the shortcut.
    let task = tokio::spawn(async move {
        let _proxy = proxy;
        let _session = session;

        while let Some(activation) = activations.next().await {
            if activation.shortcut_id() != SHORTCUT_ID {
                debug!(id = activation.shortcut_id(), "ignoring an unrelated shortcut");
                continue;
            }

            // Full channel means a fix is still running; dropping the extra press is
            // better than queueing a burst of pastes from a leaned-on key.
            if events_tx
                .try_send(HotkeyEvent { id: SHORTCUT_ID.to_string() })
                .is_err()
            {
                debug!("dropping a hotkey press: a fix is already in flight");
            }
        }

        warn!("the shortcut activation stream ended; the hotkey is no longer live");
    });

    Ok(HotkeyRegistration::new(events_rx, description, TaskGuard::from_tokio(task)))
}
