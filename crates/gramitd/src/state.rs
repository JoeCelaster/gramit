use std::sync::Mutex;
use std::time::Instant;

use gramit_core::client::BackendClient;
use gramit_core::Config;

use crate::fixloop::Selection;
use crate::notify::{Notifier, SilentNotifier};

#[derive(Debug, Default, Clone)]
pub struct Metrics {
    pub fixes_total: u64,
    pub last_error: Option<String>,
}

pub struct DaemonState {
    pub config: Config,
    pub client: BackendClient,
    /// None when the clipboard or injector could not be opened — the daemon still
    /// serves IPC so `gramit doctor` can explain why.
    pub selection: Option<Selection>,
    pub hotkey_registered: bool,
    pub hotkey_detail: Option<String>,
    /// Held for the duration of a selection fix. Two fixes at once would fight over
    /// the clipboard, so a second press is refused rather than queued.
    pub fix_gate: tokio::sync::Mutex<()>,
    /// How selection fixes reach the user. A hotkey press has no terminal, so this
    /// is the only feedback channel for that path.
    pub notifier: Box<dyn Notifier>,
    started: Instant,
    metrics: Mutex<Metrics>,
}

impl DaemonState {
    pub fn new(config: Config, client: BackendClient) -> Self {
        Self {
            config,
            client,
            selection: None,
            hotkey_registered: false,
            hotkey_detail: None,
            fix_gate: tokio::sync::Mutex::new(()),
            notifier: Box::new(SilentNotifier),
            started: Instant::now(),
            metrics: Mutex::new(Metrics::default()),
        }
    }

    pub fn with_selection(mut self, selection: Option<Selection>) -> Self {
        self.selection = selection;
        self
    }

    pub fn with_notifier(mut self, notifier: Box<dyn Notifier>) -> Self {
        self.notifier = notifier;
        self
    }

    pub fn with_hotkey(mut self, detail: Option<String>) -> Self {
        self.hotkey_registered = detail.is_some();
        self.hotkey_detail = detail;
        self
    }

    pub fn uptime_s(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    pub fn record_fix(&self) {
        let mut metrics = self.lock();
        metrics.fixes_total += 1;
        metrics.last_error = None;
    }

    pub fn record_error(&self, message: impl Into<String>) {
        self.lock().last_error = Some(message.into());
    }

    pub fn metrics(&self) -> Metrics {
        self.lock().clone()
    }

    /// A poisoned metrics lock is not worth killing the daemon over — the counters are
    /// advisory, so we recover the inner value and carry on.
    fn lock(&self) -> std::sync::MutexGuard<'_, Metrics> {
        self.metrics.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn state() -> DaemonState {
        let config = Config::default();
        let client = BackendClient::new(config.backend_url_trimmed(), Duration::from_secs(1)).unwrap();
        DaemonState::new(config, client)
    }

    #[test]
    fn counts_fixes() {
        let state = state();
        assert_eq!(state.metrics().fixes_total, 0);

        state.record_fix();
        state.record_fix();
        assert_eq!(state.metrics().fixes_total, 2);
    }

    #[test]
    fn a_successful_fix_clears_the_last_error() {
        let state = state();
        state.record_error("backend unreachable");
        assert_eq!(state.metrics().last_error.as_deref(), Some("backend unreachable"));

        state.record_fix();
        assert_eq!(state.metrics().last_error, None);
    }
}
