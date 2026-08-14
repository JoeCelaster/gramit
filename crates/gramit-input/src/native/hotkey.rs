//! Global hotkeys on Windows and macOS via `global-hotkey`.
//!
//! Both platforms constrain *which thread* may own the manager:
//!
//! - **Windows** — `GlobalHotKeyManager::new()` creates a hidden message window and
//!   receives `WM_HOTKEY` through its `WndProc`, but pumps nothing itself. The
//!   creating thread must dispatch messages, so the manager lives on the thread that
//!   runs [`crate::run_loop::pump_until`].
//! - **macOS** — Carbon dispatches hotkeys on the **main** thread's run loop.
//!
//! The manager is therefore `!Send` and stays where it was made; only the event
//! receiver crosses threads.

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::hotkey::{HotkeyEvent, HotkeyRegistration, TaskGuard, SHORTCUT_ID};
use crate::hotkey_spec::{self, Modifier};
use crate::InputError;

/// How long the forwarder waits for an event before re-checking whether to stop.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Owns the OS registration. Not `Send`: it must be dropped on the thread that made it.
pub struct Manager(#[allow(dead_code)] GlobalHotKeyManager);

/// Registers `hotkey`. **Must be called on the thread that pumps the event loop.**
pub fn register(hotkey: &str) -> Result<(Manager, HotkeyRegistration), InputError> {
    let spec = hotkey_spec::parse(hotkey)?;

    let code_name = spec.web_code().ok_or_else(|| {
        InputError::Hotkey(format!(
            "{hotkey:?} uses a key this platform cannot bind (key {:?})",
            spec.key()
        ))
    })?;
    let code = Code::from_str(&code_name)
        .map_err(|_| InputError::Hotkey(format!("unknown key {code_name:?} in {hotkey:?}")))?;

    let mut modifiers = Modifiers::empty();
    if spec.has(Modifier::Ctrl) {
        modifiers |= Modifiers::CONTROL;
    }
    if spec.has(Modifier::Alt) {
        modifiers |= Modifiers::ALT;
    }
    if spec.has(Modifier::Shift) {
        modifiers |= Modifiers::SHIFT;
    }
    if spec.has(Modifier::Super) {
        modifiers |= Modifiers::META;
    }

    let manager = GlobalHotKeyManager::new()
        .map_err(|err| InputError::Hotkey(format!("could not create the hotkey manager: {err}")))?;

    let binding = HotKey::new(Some(modifiers), code);
    let id = binding.id();

    manager.register(binding).map_err(|err| {
        InputError::Hotkey(format!(
            "could not bind {hotkey}: {err} — another application may already own it"
        ))
    })?;

    info!(hotkey, "global hotkey registered");

    let (events_tx, events_rx) = mpsc::channel(1);
    let stop = Arc::new(AtomicBool::new(false));

    // `GlobalHotKeyEvent::receiver()` is a process-wide crossbeam channel fed from the
    // platform callback, so draining it is a blocking loop — but it captures only the
    // id and the sender, never the manager, so it is free to run on another thread.
    let forwarder_stop = Arc::clone(&stop);
    std::thread::Builder::new()
        .name("gramit-hotkey".to_string())
        .spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();

            while !forwarder_stop.load(Ordering::Relaxed) {
                let event = match receiver.recv_timeout(POLL_INTERVAL) {
                    Ok(event) => event,
                    // A timeout just means no press this slice; loop and re-check.
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        warn!("the hotkey channel closed; the shortcut is no longer live");
                        break;
                    }
                };

                // Every press also produces a Released event; acting on both would run
                // each fix twice.
                if event.state != HotKeyState::Pressed || event.id != id {
                    continue;
                }

                if events_tx.try_send(HotkeyEvent { id: SHORTCUT_ID.to_string() }).is_err() {
                    debug!("dropping a hotkey press: a fix is already in flight");
                }
            }

            debug!("hotkey forwarder stopping");
        })
        .map_err(|err| {
            InputError::Hotkey(format!("could not start the hotkey forwarder: {err}"))
        })?;

    let registration =
        HotkeyRegistration::new(events_rx, hotkey.to_string(), TaskGuard::from_stop_flag(stop));

    Ok((Manager(manager), registration))
}
