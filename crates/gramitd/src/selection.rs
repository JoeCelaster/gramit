use std::sync::Arc;

use gramit_core::ipc::Response;
use tracing::{info, warn};

use crate::fixloop::{self, FixOutcome, Settings};
use crate::notify::notification_for;
use crate::state::DaemonState;

/// Runs the capture → correct → paste loop and notifies the user of the result.
///
/// Always returns an outcome rather than an error: a hotkey press has no terminal to
/// report to, so "there is no clipboard" and "a fix is already running" are outcomes
/// the user must be told about, exactly like a backend failure.
pub async fn run(state: &Arc<DaemonState>) -> FixOutcome {
    let outcome = run_inner(state).await;
    state.notifier.notify(notification_for(&outcome));
    outcome
}

async fn run_inner(state: &Arc<DaemonState>) -> FixOutcome {
    let Some(selection) = state.selection.as_ref() else {
        return FixOutcome::Failed {
            code: "SELECTION_UNAVAILABLE".to_string(),
            message: "This daemon cannot capture selections: the clipboard or keystroke \
                      injection is unavailable. Run `gramit doctor` for details."
                .to_string(),
            retryable: false,
        };
    };

    // Two fixes at once would race over one clipboard and paste into whichever window
    // happens to be focused. Refusing is better than corrupting the user's text.
    let Ok(_guard) = state.fix_gate.try_lock() else {
        info!("ignoring a selection fix: one is already running");
        return FixOutcome::Failed {
            code: "BUSY".to_string(),
            message: "A fix is already in progress.".to_string(),
            retryable: true,
        };
    };

    let settings = Settings::from(&state.config);
    let outcome = fixloop::run(selection, &state.client, &settings).await;

    match &outcome {
        FixOutcome::Replaced { changes, .. } => {
            state.record_fix();
            info!(changes, "selection replaced");
        }
        FixOutcome::AlreadyCorrect { .. } => state.record_fix(),
        FixOutcome::NoSelection => {}
        FixOutcome::Failed { code, message, .. } => {
            warn!(%code, %message, "selection fix failed");
            state.record_error(message.clone());
        }
    }

    outcome
}

/// IPC entry point: the same loop, flattened to a protocol response.
pub async fn fix_selection(state: &Arc<DaemonState>) -> Response {
    run(state).await.to_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::testing::RecordingNotifier;
    use gramit_core::client::BackendClient;
    use gramit_core::Config;
    use std::time::Duration;

    fn base_state(notifier: RecordingNotifier) -> DaemonState {
        let config =
            Config { backend_url: "http://127.0.0.1:1".to_string(), ..Config::default() };
        let client =
            BackendClient::new(config.backend_url_trimmed(), Duration::from_millis(200)).unwrap();
        DaemonState::new(config, client).with_notifier(Box::new(notifier))
    }

    fn state_without_selection(notifier: RecordingNotifier) -> Arc<DaemonState> {
        Arc::new(base_state(notifier))
    }

    fn state_with_selection(notifier: RecordingNotifier) -> Arc<DaemonState> {
        let desktop = gramit_input::fake::FakeDesktop::new().with_selection("he go");
        let selection = crate::fixloop::Selection {
            clipboard: Box::new(desktop.clipboard()),
            injector: Box::new(desktop.injector()),
        };
        Arc::new(base_state(notifier).with_selection(Some(selection)))
    }

    #[tokio::test]
    async fn a_missing_clipboard_is_reported_to_the_user_not_just_logged() {
        let notifier = RecordingNotifier::new();
        let state = state_without_selection(notifier.clone());

        let outcome = run(&state).await;

        assert!(matches!(outcome, FixOutcome::Failed { .. }));
        let sent = notifier.sent();
        assert_eq!(sent.len(), 1, "the user must be told the hotkey did nothing");
        assert!(sent[0].body.as_ref().unwrap().contains("doctor"));
    }

    #[tokio::test]
    async fn a_second_fix_is_refused_while_one_is_running() {
        let notifier = RecordingNotifier::new();
        let state = state_with_selection(notifier.clone());

        let _held = state.fix_gate.lock().await;
        let outcome = run(&state).await;

        match outcome {
            FixOutcome::Failed { code, retryable, .. } => {
                assert_eq!(code, "BUSY");
                assert!(retryable);
            }
            other => panic!("expected BUSY, got {other:?}"),
        }
        assert_eq!(notifier.sent()[0].summary, "Already fixing something");
    }

    #[tokio::test]
    async fn the_ipc_path_reports_the_same_failure() {
        let state = state_without_selection(RecordingNotifier::new());

        match fix_selection(&state).await {
            Response::Error { code, .. } => assert_eq!(code, "SELECTION_UNAVAILABLE"),
            other => panic!("expected an error, got {other:?}"),
        }
    }
}
