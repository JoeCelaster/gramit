use std::sync::Arc;

use gramit_core::config::Mode;
use gramit_core::error::BackendError;
use gramit_core::ipc::{Request, Response, StatusReport};
use gramit_core::VERSION;
use tracing::{debug, info, warn};

use crate::shutdown::Shutdown;
use crate::state::DaemonState;

pub async fn handle(request: Request, state: &Arc<DaemonState>, shutdown: &Shutdown) -> Response {
    match request {
        Request::Ping => Response::Pong {
            version: VERSION.to_string(),
            pid: std::process::id(),
            uptime_s: state.uptime_s(),
        },
        Request::Fix { text, mode } => fix(state, text, mode).await,
        Request::FixSelection => crate::selection::fix_selection(state).await,
        Request::FixClipboard => fix_clipboard(state).await,
        Request::Status => status(state).await,
        Request::Shutdown => {
            info!("shutdown requested over ipc");
            shutdown.trigger();
            Response::Ok
        }
    }
}

async fn fix(state: &Arc<DaemonState>, text: String, mode: Mode) -> Response {
    if text.trim().is_empty() {
        return BackendError::empty_text().into();
    }

    // Counted in characters, not bytes, so the limit means the same thing to a user
    // typing accented text as to one typing ASCII.
    let length = text.chars().count();
    if length > state.config.max_chars {
        return BackendError::too_long(length, state.config.max_chars).into();
    }

    debug!(chars = length, %mode, "fixing text");

    match state.client.fix(&text, mode).await {
        Ok(outcome) => {
            state.record_fix();
            info!(
                changes = outcome.changes,
                cached = outcome.cached,
                latency_ms = outcome.latency_ms,
                "fixed"
            );
            Response::Fixed {
                text: outcome.corrected,
                changed: outcome.changed,
                changes: outcome.changes,
                model: outcome.model,
                latency_ms: outcome.latency_ms,
                cached: outcome.cached,
            }
        }
        Err(err) => {
            warn!(code = %err.code, message = %err.message, "fix failed");
            state.record_error(err.message.clone());
            err.into()
        }
    }
}

/// Corrects the clipboard in place, leaving the result on the clipboard for the user
/// to paste. Nothing is typed and no selection is touched.
async fn fix_clipboard(state: &Arc<DaemonState>) -> Response {
    let Some(selection) = state.selection.as_ref() else {
        return Response::error(
            "SELECTION_UNAVAILABLE",
            "This daemon has no clipboard access. Run `gramit doctor` for details.",
            false,
        );
    };

    let text = match selection.clipboard.get_text().await {
        Ok(Some(text)) if !text.trim().is_empty() => text,
        Ok(_) => return BackendError::empty_text().into(),
        Err(err) => return Response::error(err.code(), err.to_string(), false),
    };

    let length = text.chars().count();
    if length > state.config.max_chars {
        return BackendError::too_long(length, state.config.max_chars).into();
    }

    let outcome = match state.client.fix(&text, state.config.mode).await {
        Ok(outcome) => outcome,
        Err(err) => {
            warn!(code = %err.code, "clipboard fix failed");
            state.record_error(err.message.clone());
            return err.into();
        }
    };

    if outcome.changed {
        if let Err(err) = selection.clipboard.set_text(outcome.corrected.clone()).await {
            return Response::error(err.code(), err.to_string(), false);
        }
    }

    state.record_fix();
    info!(changes = outcome.changes, "clipboard fixed");

    Response::Fixed {
        text: outcome.corrected,
        changed: outcome.changed,
        changes: outcome.changes,
        model: outcome.model,
        latency_ms: outcome.latency_ms,
        cached: outcome.cached,
    }
}

async fn status(state: &Arc<DaemonState>) -> Response {
    let metrics = state.metrics();

    let (backend_reachable, backend_has_key, backend_detail) = match state.client.health().await {
        Ok(health) if health.has_key => (true, Some(true), health.model),
        Ok(health) => (
            true,
            Some(false),
            Some(format!("backend has no API key; missing: {}", health.missing.join(", "))),
        ),
        Err(err) => (false, None, Some(err.message)),
    };

    Response::Status(Box::new(StatusReport {
        version: VERSION.to_string(),
        pid: std::process::id(),
        uptime_s: state.uptime_s(),
        hotkey: state.config.hotkey.clone(),
        hotkey_registered: state.hotkey_registered,
        hotkey_detail: state.hotkey_detail.clone(),
        selection_ready: state.selection.is_some(),
        injector: state.selection.as_ref().map(|s| s.describe()),
        backend_url: state.config.backend_url_trimmed().to_string(),
        backend_reachable,
        backend_has_key,
        backend_detail,
        mode: state.config.mode,
        notifications: state.config.notifications,
        fixes_total: metrics.fixes_total,
        last_error: metrics.last_error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gramit_core::client::BackendClient;
    use gramit_core::Config;
    use std::time::Duration;

    /// Points the client at a port nothing listens on, so backend calls fail fast.
    fn state_with_dead_backend(max_chars: usize) -> Arc<DaemonState> {
        let config = Config {
            backend_url: "http://127.0.0.1:1".to_string(),
            max_chars,
            ..Config::default()
        };
        let client =
            BackendClient::new(config.backend_url_trimmed(), Duration::from_millis(500)).unwrap();
        Arc::new(DaemonState::new(config, client))
    }

    #[tokio::test]
    async fn ping_reports_version_and_pid() {
        let state = state_with_dead_backend(100);
        let shutdown = Shutdown::new();

        match handle(Request::Ping, &state, &shutdown).await {
            Response::Pong { version, pid, .. } => {
                assert_eq!(version, VERSION);
                assert_eq!(pid, std::process::id());
            }
            other => panic!("expected pong, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_text_is_rejected_without_calling_the_backend() {
        let state = state_with_dead_backend(100);
        let shutdown = Shutdown::new();
        let request = Request::Fix { text: "   \n ".into(), mode: Mode::Grammar };

        match handle(request, &state, &shutdown).await {
            Response::Error { code, .. } => assert_eq!(code, "EMPTY_TEXT"),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn oversized_text_is_rejected_before_the_request() {
        let state = state_with_dead_backend(10);
        let shutdown = Shutdown::new();
        let request = Request::Fix { text: "a".repeat(11), mode: Mode::Grammar };

        match handle(request, &state, &shutdown).await {
            Response::Error { code, message, .. } => {
                assert_eq!(code, "TOO_LONG");
                assert!(message.contains("11"), "{message}");
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_limit_counts_characters_not_bytes() {
        // 5 characters, 10 bytes in UTF-8 — must pass a 5-character limit.
        let state = state_with_dead_backend(5);
        let shutdown = Shutdown::new();
        let request = Request::Fix { text: "éèêëî".into(), mode: Mode::Grammar };

        match handle(request, &state, &shutdown).await {
            // Reaches the backend (and fails there), which proves the length check passed.
            Response::Error { code, .. } => assert_eq!(code, "BACKEND_UNREACHABLE"),
            other => panic!("expected a backend error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unreachable_backend_is_reported_and_recorded() {
        let state = state_with_dead_backend(100);
        let shutdown = Shutdown::new();
        let request = Request::Fix { text: "he go".into(), mode: Mode::Grammar };

        match handle(request, &state, &shutdown).await {
            Response::Error { code, retryable, .. } => {
                assert_eq!(code, "BACKEND_UNREACHABLE");
                assert!(retryable);
            }
            other => panic!("expected an error, got {other:?}"),
        }
        assert!(state.metrics().last_error.is_some());
        assert_eq!(state.metrics().fixes_total, 0);
    }

    #[tokio::test]
    async fn status_reports_an_unreachable_backend() {
        let state = state_with_dead_backend(100);
        let shutdown = Shutdown::new();

        match handle(Request::Status, &state, &shutdown).await {
            Response::Status(report) => {
                assert!(!report.backend_reachable);
                assert_eq!(report.backend_has_key, None);
                assert!(!report.hotkey_registered);
                assert_eq!(report.hotkey, "Ctrl+Alt+F");
            }
            other => panic!("expected status, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fix_selection_explains_itself_without_selection_machinery() {
        // This state has no clipboard or injector, as on a headless box.
        let state = state_with_dead_backend(100);
        let shutdown = Shutdown::new();

        match handle(Request::FixSelection, &state, &shutdown).await {
            Response::Error { code, message, .. } => {
                assert_eq!(code, "SELECTION_UNAVAILABLE");
                assert!(message.contains("doctor"), "{message}");
            }
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn shutdown_triggers_and_acknowledges() {
        let state = state_with_dead_backend(100);
        let shutdown = Shutdown::new();

        assert_eq!(handle(Request::Shutdown, &state, &shutdown).await, Response::Ok);
        assert!(shutdown.is_triggered());
    }
}
