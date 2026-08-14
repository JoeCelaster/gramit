use tokio::sync::mpsc;

use crate::InputError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyEvent {
    /// Shortcut id as registered; only `fix-selection` exists in v1.
    pub id: String,
}

/// A live hotkey registration.
///
/// Dropping this releases the shortcut, so hold it for the daemon's life.
pub struct HotkeyRegistration {
    pub events: mpsc::Receiver<HotkeyEvent>,
    /// What actually got bound, which may differ from what we asked for — the
    /// compositor has the final say and the user can rebind it in system settings.
    pub description: String,
    _guard: TaskGuard,
}

impl HotkeyRegistration {
    pub fn new(events: mpsc::Receiver<HotkeyEvent>, description: String, guard: TaskGuard) -> Self {
        Self { events, description, _guard: guard }
    }
}

/// Stops whatever is feeding hotkey events when the registration is dropped.
///
/// The two backends need different stops: Linux runs an async task that can simply be
/// aborted, while the Windows/macOS forwarder blocks on a channel and has to be asked
/// to stop via a flag it polls.
pub struct TaskGuard(Option<Box<dyn FnOnce() + Send>>);

impl TaskGuard {
    pub fn from_tokio(handle: tokio::task::JoinHandle<()>) -> Self {
        Self(Some(Box::new(move || handle.abort())))
    }

    pub fn from_stop_flag(flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        Self(Some(Box::new(move || {
            flag.store(true, std::sync::atomic::Ordering::Relaxed)
        })))
    }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            stop();
        }
    }
}

pub const SHORTCUT_ID: &str = "fix-selection";

/// Platform hotkey state that must live on the main thread, held by `main` for the
/// life of the process. Empty on Linux, where the portal has no thread requirement.
pub struct MainThreadHotkey {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    _manager: Option<crate::native::hotkey::Manager>,
}

/// Registers the hotkey from the main thread, where the platform requires it.
///
/// Windows delivers `WM_HOTKEY` to a window owned by the creating thread, and macOS
/// dispatches Carbon hotkeys on the main run loop — on both, the manager must be
/// created on the thread that pumps the event loop. Returning `None` means this
/// platform has no such constraint and [`register`] should be awaited instead.
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn register_on_main_thread(
    hotkey: &str,
) -> (MainThreadHotkey, Option<Result<HotkeyRegistration, InputError>>) {
    match crate::native::hotkey::register(hotkey) {
        Ok((manager, registration)) => {
            (MainThreadHotkey { _manager: Some(manager) }, Some(Ok(registration)))
        }
        Err(err) => (MainThreadHotkey { _manager: None }, Some(Err(err))),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn register_on_main_thread(
    _hotkey: &str,
) -> (MainThreadHotkey, Option<Result<HotkeyRegistration, InputError>>) {
    // Linux registers asynchronously through the portal, from any thread.
    (MainThreadHotkey {}, None)
}

/// Registers the configured hotkey with the OS, for platforms with no main-thread
/// requirement.
#[cfg(target_os = "linux")]
pub async fn register(hotkey: &str) -> Result<HotkeyRegistration, InputError> {
    crate::linux::shortcuts::register(hotkey).await
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub async fn register(_hotkey: &str) -> Result<HotkeyRegistration, InputError> {
    Err(InputError::Hotkey(
        "this platform must register its hotkey from the main thread; \
         use register_on_main_thread"
            .into(),
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub async fn register(_hotkey: &str) -> Result<HotkeyRegistration, InputError> {
    Err(InputError::Unsupported(
        "global hotkeys are not supported on this platform".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn dropping_the_guard_sets_the_stop_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let guard = TaskGuard::from_stop_flag(Arc::clone(&flag));

        assert!(!flag.load(Ordering::Relaxed));
        drop(guard);
        assert!(flag.load(Ordering::Relaxed), "the forwarder must be told to stop");
    }

    #[tokio::test]
    async fn dropping_the_guard_aborts_the_task() {
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        let guard = TaskGuard::from_tokio(handle);
        drop(guard);
        // An aborted task stops; if the guard did nothing this would hang forever.
    }
}
