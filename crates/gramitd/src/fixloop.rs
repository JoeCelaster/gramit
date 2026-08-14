//! The capture → correct → paste loop: what actually happens when the user hits the
//! hotkey with text selected.
//!
//! Everything here works against the `Clipboard`, `Injector`, and `Corrector` traits,
//! so the whole sequence — including its failure paths — is tested without a desktop.

use std::time::Duration;

use async_trait::async_trait;
use gramit_core::client::{BackendClient, FixOutcome as BackendFix};
use gramit_core::config::Mode;
use gramit_core::error::BackendError;
use gramit_core::ipc::Response;
use gramit_core::{Capture, Config};
use gramit_input::clipboard::{self, Clipboard};
use gramit_input::injector::Injector;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// The text-correcting half of the loop, abstracted so tests don't need HTTP.
#[async_trait]
pub trait Corrector: Send + Sync {
    async fn correct(&self, text: &str, mode: Mode) -> Result<BackendFix, BackendError>;
}

#[async_trait]
impl Corrector for BackendClient {
    async fn correct(&self, text: &str, mode: Mode) -> Result<BackendFix, BackendError> {
        self.fix(text, mode).await
    }
}

/// The platform machinery needed to replace a selection.
pub struct Selection {
    pub clipboard: Box<dyn Clipboard>,
    pub injector: Box<dyn Injector>,
}

impl Selection {
    pub fn describe(&self) -> String {
        self.injector.describe()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FixOutcome {
    /// The selection was corrected and pasted over.
    Replaced { text: String, changes: u32, model: String, latency_ms: u64, cached: bool },
    /// The text was already correct, so nothing was pasted.
    AlreadyCorrect { text: String, model: String, latency_ms: u64, cached: bool },
    /// The copy produced nothing — most likely nothing was selected.
    NoSelection,
    Failed { code: String, message: String, retryable: bool },
}

impl FixOutcome {
    fn failed(code: &str, message: impl Into<String>, retryable: bool) -> Self {
        FixOutcome::Failed { code: code.to_string(), message: message.into(), retryable }
    }

    pub fn to_response(&self) -> Response {
        match self {
            FixOutcome::Replaced { text, changes, model, latency_ms, cached } => Response::Fixed {
                text: text.clone(),
                changed: true,
                changes: *changes,
                model: model.clone(),
                latency_ms: *latency_ms,
                cached: *cached,
            },
            FixOutcome::AlreadyCorrect { text, model, latency_ms, cached } => Response::Fixed {
                text: text.clone(),
                changed: false,
                changes: 0,
                model: model.clone(),
                latency_ms: *latency_ms,
                cached: *cached,
            },
            FixOutcome::NoSelection => Response::error(
                "NO_SELECTION",
                "Nothing was selected, so there was nothing to correct.",
                false,
            ),
            FixOutcome::Failed { code, message, retryable } => {
                Response::error(code.clone(), message.clone(), *retryable)
            }
        }
    }
}

pub struct Settings {
    pub mode: Mode,
    pub capture: Capture,
    pub max_chars: usize,
    pub modifier_release: Duration,
    /// Total budget for landing the copy, across retries.
    pub copy_settle: Duration,
    /// How long to watch the clipboard before injecting another Ctrl+C.
    pub copy_retry_interval: Duration,
    pub paste_delay: Duration,
    pub restore_delay: Duration,
    pub poll_interval: Duration,
}

impl From<&Config> for Settings {
    fn from(config: &Config) -> Self {
        Self {
            mode: config.mode,
            capture: config.capture,
            max_chars: config.max_chars,
            modifier_release: Duration::from_millis(config.modifier_release_ms),
            copy_settle: Duration::from_millis(config.copy_settle_ms),
            copy_retry_interval: Duration::from_millis(config.copy_retry_interval_ms),
            paste_delay: Duration::from_millis(config.paste_delay_ms),
            restore_delay: Duration::from_millis(config.restore_delay_ms),
            poll_interval: Duration::from_millis(25),
        }
    }
}

/// Runs one full fix against whatever the user currently has selected.
pub async fn run(
    selection: &Selection,
    corrector: &dyn Corrector,
    settings: &Settings,
) -> FixOutcome {
    let clipboard = selection.clipboard.as_ref();

    let saved = match clipboard::snapshot(clipboard).await {
        Ok(saved) => saved,
        Err(err) => return FixOutcome::failed(err.code(), err.to_string(), false),
    };

    let outcome = capture_and_replace(selection, corrector, settings).await;

    // Whatever happened, the user's clipboard goes back to what it was.
    if let Err(err) = clipboard::restore(clipboard, &saved).await {
        warn!(%err, "could not restore the clipboard");
    }

    outcome
}

async fn capture_and_replace(
    selection: &Selection,
    corrector: &dyn Corrector,
    settings: &Settings,
) -> FixOutcome {
    let clipboard = selection.clipboard.as_ref();

    let captured = match capture(selection, settings).await {
        Ok(Some(text)) => text,
        Ok(None) => return FixOutcome::NoSelection,
        Err(err) => return FixOutcome::failed(err.code(), err.to_string(), false),
    };

    if captured.trim().is_empty() {
        return FixOutcome::NoSelection;
    }

    let length = captured.chars().count();
    if length > settings.max_chars {
        let err = BackendError::too_long(length, settings.max_chars);
        return FixOutcome::failed(&err.code, err.message, false);
    }

    info!(chars = length, "captured selection");

    let corrected = match corrector.correct(&captured, settings.mode).await {
        Ok(outcome) => outcome,
        Err(err) => return FixOutcome::failed(&err.code, err.message, err.retryable),
    };

    // Pasting identical text would still cost the user an undo step and scroll the
    // caret, so leave the selection alone when there's nothing to change.
    if !corrected.changed || corrected.corrected == captured {
        info!("selection was already correct");
        return FixOutcome::AlreadyCorrect {
            text: corrected.corrected,
            model: corrected.model,
            latency_ms: corrected.latency_ms,
            cached: corrected.cached,
        };
    }

    if let Err(err) = clipboard.set_text(corrected.corrected.clone()).await {
        return FixOutcome::failed(err.code(), err.to_string(), false);
    }

    // Give the compositor time to publish the new clipboard contents before the
    // target app asks for them.
    sleep(settings.paste_delay).await;

    if let Err(err) = selection.injector.paste().await {
        return FixOutcome::failed(err.code(), err.to_string(), false);
    }

    // The paste is asynchronous: the app requests the clipboard after receiving the
    // keystroke, so restoring immediately would hand it the old text.
    sleep(settings.restore_delay).await;

    info!(changes = corrected.changes, "replaced selection");

    FixOutcome::Replaced {
        text: corrected.corrected,
        changes: corrected.changes,
        model: corrected.model,
        latency_ms: corrected.latency_ms,
        cached: corrected.cached,
    }
}

/// Reads whatever the user has selected.
async fn capture(
    selection: &Selection,
    settings: &Settings,
) -> Result<Option<String>, gramit_input::InputError> {
    let clipboard = selection.clipboard.as_ref();

    if settings.capture == Capture::Primary {
        // No keystroke at all: selecting text already published it.
        let text = clipboard.get_primary_text().await?;
        if text.as_deref().is_none_or(str::is_empty) {
            info!("PRIMARY selection was empty");
            return Ok(None);
        }
        return Ok(text);
    }

    // The hotkey's own modifiers are very likely still held — GNOME fires a keybinding
    // on key *press*, and a Ctrl+Alt+F chord is commonly held for 300-500ms. Anything
    // injected before the user lets go arrives as Ctrl+Alt+C and copies nothing, so we
    // wait, then keep retrying rather than judging the selection empty on one attempt.
    sleep(settings.modifier_release).await;

    // Emptying the clipboard first turns "did the copy work?" into "is there anything
    // here?". Comparing against the previous contents instead would mistake a
    // selection identical to the current clipboard for a failed copy.
    clipboard.clear().await?;

    let started = tokio::time::Instant::now();
    let mut attempts = 0u32;

    loop {
        attempts += 1;
        selection.injector.copy().await?;

        if let Some(text) = poll_clipboard(clipboard, settings, settings.copy_retry_interval).await?
        {
            if attempts > 1 {
                debug!(attempts, "the copy landed on a retry");
            }
            return Ok(Some(text));
        }

        if started.elapsed() >= settings.copy_settle {
            // Whether PRIMARY has text is the tell: if the user clearly has a selection
            // and our copy still produced nothing, the keystroke is not reaching the
            // app — a different problem from "nothing was selected".
            let primary_has_text = clipboard
                .get_primary_text()
                .await
                .ok()
                .flatten()
                .is_some_and(|text| !text.trim().is_empty());

            info!(
                attempts,
                elapsed_ms = started.elapsed().as_millis() as u64,
                primary_has_text,
                "no text captured{}",
                if primary_has_text {
                    " — PRIMARY has a selection, so the injected Ctrl+C is not reaching the app"
                } else {
                    " — nothing appears to be selected"
                }
            );
            return Ok(None);
        }
    }
}

/// Polls until the clipboard holds text, or the settle window expires.
async fn poll_clipboard(
    clipboard: &dyn Clipboard,
    settings: &Settings,
    window: Duration,
) -> Result<Option<String>, gramit_input::InputError> {
    let deadline = tokio::time::Instant::now() + window;

    loop {
        if let Some(text) = clipboard.get_text().await? {
            if !text.is_empty() {
                return Ok(Some(text));
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        sleep(settings.poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gramit_input::fake::{Action, FakeDesktop};

    struct FakeCorrector {
        result: Result<BackendFix, BackendError>,
    }

    impl FakeCorrector {
        fn returning(corrected: &str, changes: u32) -> Self {
            Self {
                result: Ok(BackendFix {
                    corrected: corrected.to_string(),
                    changed: changes > 0,
                    changes,
                    model: "gpt-5.6-luna".into(),
                    latency_ms: 42,
                    cached: false,
                }),
            }
        }

        fn failing(err: BackendError) -> Self {
            Self { result: Err(err) }
        }
    }

    #[async_trait]
    impl Corrector for FakeCorrector {
        async fn correct(&self, _text: &str, _mode: Mode) -> Result<BackendFix, BackendError> {
            self.result.clone()
        }
    }

    fn settings() -> Settings {
        Settings::from(&Config::default())
    }

    fn selection_from(desktop: &FakeDesktop) -> Selection {
        Selection {
            clipboard: Box::new(desktop.clipboard()),
            injector: Box::new(desktop.injector()),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn replaces_the_selection_and_restores_the_clipboard() {
        let desktop = FakeDesktop::new()
            .with_clipboard("something the user copied earlier")
            .with_selection("he go to the store");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("He goes to the store.", 2);

        let outcome = run(&selection, &corrector, &settings()).await;

        assert_eq!(
            outcome,
            FixOutcome::Replaced {
                text: "He goes to the store.".into(),
                changes: 2,
                model: "gpt-5.6-luna".into(),
                latency_ms: 42,
                cached: false,
            }
        );
        assert_eq!(desktop.actions(), vec![Action::Copy, Action::Paste]);
        assert_eq!(
            desktop.clipboard_contents().as_deref(),
            Some("something the user copied earlier"),
            "the user's clipboard must be put back"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn does_not_paste_when_the_text_is_already_correct() {
        let desktop = FakeDesktop::new().with_selection("He goes to the store.");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("He goes to the store.", 0);

        let outcome = run(&selection, &corrector, &settings()).await;

        assert!(matches!(outcome, FixOutcome::AlreadyCorrect { .. }));
        assert_eq!(desktop.actions(), vec![Action::Copy], "nothing should have been pasted");
    }

    #[tokio::test(start_paused = true)]
    async fn treats_an_unchanged_correction_as_already_correct() {
        // The backend can report changed=true while returning identical text; the
        // paste is still pointless.
        let desktop = FakeDesktop::new().with_selection("Same text.");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("Same text.", 3);

        let outcome = run(&selection, &corrector, &settings()).await;

        assert!(matches!(outcome, FixOutcome::AlreadyCorrect { .. }));
        assert_eq!(desktop.actions(), vec![Action::Copy]);
    }

    #[tokio::test(start_paused = true)]
    async fn reports_no_selection_when_the_copy_yields_nothing() {
        let desktop = FakeDesktop::new().with_clipboard("earlier clipboard text");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("unused", 1);

        let outcome = run(&selection, &corrector, &settings()).await;

        assert_eq!(outcome, FixOutcome::NoSelection);
        // The copy is retried across the settle window, but nothing is ever pasted.
        assert!(desktop.copy_attempts() >= 1);
        assert!(
            !desktop.actions().contains(&Action::Paste),
            "nothing should be pasted when there is no selection"
        );
        assert_eq!(
            desktop.clipboard_contents().as_deref(),
            Some("earlier clipboard text"),
            "a failed capture must still restore the clipboard"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn detects_a_selection_identical_to_the_current_clipboard() {
        // Clearing before the copy is what makes this work; comparing against the
        // previous contents would read this as "nothing selected".
        let desktop = FakeDesktop::new()
            .with_clipboard("he go to the store")
            .with_selection("he go to the store");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("He goes to the store.", 2);

        let outcome = run(&selection, &corrector, &settings()).await;

        assert!(matches!(outcome, FixOutcome::Replaced { .. }));
        assert_eq!(desktop.actions(), vec![Action::Copy, Action::Paste]);
    }

    #[tokio::test(start_paused = true)]
    async fn whitespace_only_selections_are_not_sent_to_the_backend() {
        let desktop = FakeDesktop::new().with_selection("   \n  ");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::failing(BackendError::unreachable("http://x", "should not be called"));

        assert_eq!(run(&selection, &corrector, &settings()).await, FixOutcome::NoSelection);
    }

    #[tokio::test(start_paused = true)]
    async fn oversized_selections_are_refused_before_the_backend() {
        let desktop = FakeDesktop::new().with_selection(&"a".repeat(50));
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("unused", 1);

        let mut settings = settings();
        settings.max_chars = 10;

        match run(&selection, &corrector, &settings).await {
            FixOutcome::Failed { code, .. } => assert_eq!(code, "TOO_LONG"),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(desktop.actions(), vec![Action::Copy], "nothing should have been pasted");
    }

    #[tokio::test(start_paused = true)]
    async fn a_backend_failure_keeps_its_code_and_restores_the_clipboard() {
        let desktop = FakeDesktop::new()
            .with_clipboard("earlier")
            .with_selection("he go");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::failing(BackendError::new("NO_API_KEY", "not configured", false));

        match run(&selection, &corrector, &settings()).await {
            FixOutcome::Failed { code, retryable, .. } => {
                assert_eq!(code, "NO_API_KEY");
                assert!(!retryable);
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(desktop.clipboard_contents().as_deref(), Some("earlier"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_copy_is_reported_and_the_clipboard_restored() {
        let desktop = FakeDesktop::new()
            .with_clipboard("earlier")
            .with_selection("he go")
            .failing_copy("the portal session is gone");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("unused", 1);

        match run(&selection, &corrector, &settings()).await {
            FixOutcome::Failed { code, .. } => assert_eq!(code, "INJECTION_ERROR"),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(desktop.clipboard_contents().as_deref(), Some("earlier"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_paste_is_reported_and_the_clipboard_restored() {
        let desktop = FakeDesktop::new()
            .with_clipboard("earlier")
            .with_selection("he go")
            .failing_paste("the portal session is gone");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("He goes.", 1);

        match run(&selection, &corrector, &settings()).await {
            FixOutcome::Failed { code, .. } => assert_eq!(code, "INJECTION_ERROR"),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(
            desktop.clipboard_contents().as_deref(),
            Some("earlier"),
            "a failed paste must not leave the correction on the clipboard"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retries_the_copy_while_the_hotkey_is_still_held() {
        // The reported bug: GNOME fires the keybinding on key press, so the first
        // injected Ctrl+C arrives as Ctrl+Alt+C and copies nothing.
        let desktop = FakeDesktop::new()
            .with_selection("he go to the store")
            .copy_succeeds_on_attempt(3);
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("He goes to the store.", 2);

        let outcome = run(&selection, &corrector, &settings()).await;

        assert!(
            matches!(outcome, FixOutcome::Replaced { .. }),
            "a copy that lands on a later attempt must still fix the text, got {outcome:?}"
        );
        assert_eq!(desktop.copy_attempts(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_working_copy_is_not_retried() {
        let desktop = FakeDesktop::new().with_selection("he go");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("He goes.", 1);

        run(&selection, &corrector, &settings()).await;

        assert_eq!(desktop.copy_attempts(), 1, "a successful first copy must cost nothing extra");
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_after_the_settle_budget() {
        // Nothing is selected, so no number of retries will help.
        let desktop = FakeDesktop::new();
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("unused", 1);

        let outcome = run(&selection, &corrector, &settings()).await;

        assert_eq!(outcome, FixOutcome::NoSelection);
        let attempts = desktop.copy_attempts();
        assert!((2..=12).contains(&attempts), "expected a few retries, got {attempts}");
    }

    #[tokio::test(start_paused = true)]
    async fn primary_capture_never_injects_a_copy() {
        let desktop = FakeDesktop::new().with_primary("he go to the store");
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("He goes to the store.", 2);

        let mut settings = settings();
        settings.capture = Capture::Primary;

        let outcome = run(&selection, &corrector, &settings).await;

        assert!(matches!(outcome, FixOutcome::Replaced { .. }));
        assert_eq!(desktop.copy_attempts(), 0, "PRIMARY needs no keystroke");
        assert_eq!(desktop.actions(), vec![Action::Paste]);
    }

    #[tokio::test(start_paused = true)]
    async fn primary_capture_reports_an_empty_selection() {
        let desktop = FakeDesktop::new();
        let selection = selection_from(&desktop);
        let corrector = FakeCorrector::returning("unused", 1);

        let mut settings = settings();
        settings.capture = Capture::Primary;

        assert_eq!(run(&selection, &corrector, &settings).await, FixOutcome::NoSelection);
    }

    #[tokio::test(start_paused = true)]
    async fn outcomes_map_onto_ipc_responses() {
        let replaced = FixOutcome::Replaced {
            text: "He goes.".into(),
            changes: 2,
            model: "m".into(),
            latency_ms: 1,
            cached: false,
        };
        match replaced.to_response() {
            Response::Fixed { changed, changes, .. } => {
                assert!(changed);
                assert_eq!(changes, 2);
            }
            other => panic!("unexpected response {other:?}"),
        }

        match FixOutcome::NoSelection.to_response() {
            Response::Error { code, .. } => assert_eq!(code, "NO_SELECTION"),
            other => panic!("unexpected response {other:?}"),
        }
    }
}
