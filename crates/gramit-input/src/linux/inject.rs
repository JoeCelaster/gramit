use ashpd::desktop::remote_desktop::{
    DeviceType, KeyState, NotifyKeyboardKeysymOptions, RemoteDesktop, SelectDevicesOptions,
    StartOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, Session};
use ashpd::enumflags2::BitFlags;
use async_trait::async_trait;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::keys::{KEYSYM_C, KEYSYM_CONTROL_L, KEYSYM_V, MODIFIER_KEYSYMS};
use super::token;
use crate::injector::Injector;
use crate::InputError;

fn portal_error(context: &str, err: ashpd::Error) -> InputError {
    InputError::Portal(format!("{context}: {err}"))
}

/// Whether an error means the portal session is gone rather than the request being bad.
///
/// Sessions die for reasons outside our control — the portal service restarts, the user
/// revokes access, another client takes over the persisted session. Without this the
/// daemon would fail every fix until someone ran `gramit restart`.
fn is_session_gone(err: &InputError) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    message.contains("invalid session")
        || message.contains("session is closed")
        || message.contains("session does not exist")
        || (message.contains("session") && message.contains("does not exist"))
}

/// A live portal session. Replaced wholesale when it dies.
struct Connected {
    proxy: RemoteDesktop,
    session: Session<RemoteDesktop>,
}

impl Connected {
    async fn establish() -> Result<Self, InputError> {
        match Self::try_establish(token::load()).await {
            Ok(connected) => Ok(connected),
            // A saved token the portal no longer honours (revoked in Settings, or a
            // portal restart) fails the same way as no permission at all. Drop it and
            // ask once, rather than leaving the user stuck with a broken hotkey.
            Err(err) if token::load().is_some() => {
                warn!(%err, "the saved portal token was rejected; asking for permission again");
                token::clear();
                Self::try_establish(None).await
            }
            Err(err) => Err(err),
        }
    }

    async fn try_establish(restore_token: Option<String>) -> Result<Self, InputError> {
        let proxy = RemoteDesktop::new()
            .await
            .map_err(|err| portal_error("could not reach the RemoteDesktop portal", err))?;

        let session = proxy
            .create_session(CreateSessionOptions::default())
            .await
            .map_err(|err| portal_error("could not create a RemoteDesktop session", err))?;

        proxy
            .select_devices(
                &session,
                SelectDevicesOptions::default()
                    .set_devices(BitFlags::from(DeviceType::Keyboard))
                    // Persist so the consent dialog appears once, not every start.
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(restore_token.as_deref()),
            )
            .await
            .map_err(|err| portal_error("could not request keyboard access", err))?
            .response()
            .map_err(|err| portal_error("keyboard access was refused", err))?;

        let selected = proxy
            .start(&session, None, StartOptions::default())
            .await
            .map_err(|err| portal_error("could not start the RemoteDesktop session", err))?
            .response()
            .map_err(|err| portal_error("the RemoteDesktop session was not allowed", err))?;

        if let Some(new_token) = selected.restore_token() {
            token::save(new_token);
        }

        if !selected.devices().contains(DeviceType::Keyboard) {
            return Err(InputError::Portal(
                "the portal granted a session without keyboard access".into(),
            ));
        }

        info!("RemoteDesktop portal session established");
        Ok(Self { proxy, session })
    }

    async fn key(&self, keysym: i32, state: KeyState) -> Result<(), InputError> {
        self.proxy
            .notify_keyboard_keysym(
                &self.session,
                keysym,
                state,
                NotifyKeyboardKeysymOptions::default(),
            )
            .await
            .map_err(|err| portal_error("could not send a keystroke", err))
    }

    /// Releases every modifier before we type.
    ///
    /// The hotkey that triggered this fix is very likely still held down — GNOME fires
    /// a keybinding on key *press*, and a `Ctrl+Alt+F` chord is comfortably held for
    /// several hundred milliseconds. Without this, the `Ctrl+C` we inject arrives as
    /// `Ctrl+Alt+C`, which copies nothing and looks exactly like an empty selection.
    async fn release_modifiers(&self) -> Result<(), InputError> {
        for keysym in MODIFIER_KEYSYMS {
            self.key(keysym, KeyState::Released).await?;
        }
        Ok(())
    }

    /// Presses Ctrl+<key> and releases both, in the order a real keyboard would.
    async fn ctrl_chord(&self, keysym: i32) -> Result<(), InputError> {
        self.release_modifiers().await?;

        self.key(KEYSYM_CONTROL_L, KeyState::Pressed).await?;

        let pressed = self.key(keysym, KeyState::Pressed).await;
        let released = self.key(keysym, KeyState::Released).await;

        // Release Ctrl no matter what: leaving a stuck modifier would wreck every
        // subsequent keystroke the user types.
        let ctrl_released = self.key(KEYSYM_CONTROL_L, KeyState::Released).await;

        pressed?;
        released?;
        ctrl_released?;
        debug!(keysym, "sent ctrl chord");
        Ok(())
    }
}

/// Injects keystrokes through the RemoteDesktop portal.
///
/// The session must outlive every injection, so this struct owns it — and replaces it
/// if the portal drops it underneath us.
pub struct PortalInjector {
    connected: Mutex<Connected>,
}

impl PortalInjector {
    pub async fn connect() -> Result<Self, InputError> {
        Ok(Self { connected: Mutex::new(Connected::establish().await?) })
    }

    async fn chord(&self, keysym: i32) -> Result<(), InputError> {
        let mut guard = self.connected.lock().await;

        match guard.ctrl_chord(keysym).await {
            Ok(()) => Ok(()),
            Err(err) if is_session_gone(&err) => {
                warn!(%err, "the portal session is gone; reconnecting");
                *guard = Connected::establish().await?;
                guard.ctrl_chord(keysym).await
            }
            Err(err) => Err(err),
        }
    }
}

#[async_trait]
impl Injector for PortalInjector {
    async fn copy(&self) -> Result<(), InputError> {
        self.chord(KEYSYM_C).await
    }

    async fn paste(&self) -> Result<(), InputError> {
        self.chord(KEYSYM_V).await
    }

    fn describe(&self) -> String {
        "RemoteDesktop portal (Wayland)".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_dead_session() {
        let err = InputError::Portal(
            "could not send a keystroke: Portal request failed: \
             org.freedesktop.zbus.Error: Invalid session"
                .into(),
        );
        assert!(is_session_gone(&err));
    }

    #[test]
    fn does_not_mistake_other_failures_for_a_dead_session() {
        // Reconnecting would not help here, and would hide the real problem.
        let err = InputError::Portal("keyboard access was refused".into());
        assert!(!is_session_gone(&err));

        let err = InputError::Injection("the injector thread is gone".into());
        assert!(!is_session_gone(&err));
    }
}
