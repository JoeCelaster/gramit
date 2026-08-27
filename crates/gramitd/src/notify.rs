//! Desktop notifications — the only feedback channel for a hotkey fix.
//!
//! A selection fix has no terminal to report to: the user pressed a key in some other
//! app and the text either changed or it didn't. Every outcome therefore gets a
//! toast, including the boring ones, so a silent failure is never mistaken for
//! "nothing needed fixing".

use gramit_core::config::Mode;
use gramit_core::Config;
use tracing::{debug, warn};

use crate::fixloop::FixOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub summary: String,
    pub body: Option<String>,
    pub urgency: Urgency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Low,
    Normal,
    Critical,
}

pub trait Notifier: Send + Sync {
    fn notify(&self, notification: Notification);
}

/// Builds the toast for an outcome. Pure, so the wording is testable.
///
/// The mode decides the words. "Fixed 3 issues" is right for prose and wrong for code,
/// where a word diff counts a renamed variable once per use; "Code updated" is right
/// for code and says nothing to somebody correcting an email.
pub fn notification_for(outcome: &FixOutcome, mode: Mode) -> Notification {
    match outcome {
        FixOutcome::Replaced { changes, .. } => Notification {
            summary: match mode {
                Mode::Code => "Code updated".to_string(),
                Mode::Grammar => match changes {
                    0 | 1 => "Fixed 1 issue".to_string(),
                    n => format!("Fixed {n} issues"),
                },
            },
            // In code mode the count is detail rather than headline, because it
            // overstates small edits. In grammar mode the summary already carries it.
            body: match mode {
                Mode::Code => Some(match changes {
                    0 | 1 => "1 edit pasted over your selection.".to_string(),
                    n => format!("{n} edits pasted over your selection."),
                }),
                Mode::Grammar => None,
            },
            urgency: Urgency::Low,
        },
        FixOutcome::AlreadyCorrect { .. } => Notification {
            summary: match mode {
                Mode::Code => "No changes needed".to_string(),
                Mode::Grammar => "Looks good already".to_string(),
            },
            body: Some(match mode {
                Mode::Code => "The code already does what its comments ask for.".to_string(),
                Mode::Grammar => "No changes needed.".to_string(),
            }),
            urgency: Urgency::Low,
        },
        FixOutcome::NoSelection => Notification {
            summary: "Nothing selected".to_string(),
            body: Some(match mode {
                Mode::Code => "Select some code, then press the hotkey.".to_string(),
                Mode::Grammar => "Select some text, then press the hotkey.".to_string(),
            }),
            urgency: Urgency::Normal,
        },
        FixOutcome::Failed { code, message, .. } => Notification {
            summary: summary_for_code(code).to_string(),
            body: Some(message.clone()),
            urgency: Urgency::Critical,
        },
    }
}

/// Turns an error code into something a user can act on, rather than showing them a
/// raw code. The code still travels in the body via `message`.
fn summary_for_code(code: &str) -> &'static str {
    match code {
        "NO_API_KEY" => "gramit backend has no API key",
        "BACKEND_UNREACHABLE" => "gramit backend is not running",
        "BACKEND_TIMEOUT" | "UPSTREAM_TIMEOUT" => "Timed out",
        "RATE_LIMITED" => "Rate limited",
        "TOO_LONG" => "Selection too long",
        "BUSY" => "Already fixing something",
        "INJECTION_ERROR" | "PORTAL_ERROR" => "Could not paste the result",
        "CLIPBOARD_ERROR" => "Could not read the clipboard",
        _ => "Could not fix that",
    }
}

/// Sends real desktop notifications.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&self, notification: Notification) {
        // notify-rust talks D-Bus (or the OS notification centre) synchronously, so
        // it must not run on a runtime worker.
        std::thread::spawn(move || {
            let mut builder = notify_rust::Notification::new();
            builder.appname("gramit").summary(&notification.summary);

            if let Some(body) = &notification.body {
                builder.body(body);
            }

            #[cfg(all(unix, not(target_os = "macos")))]
            builder.urgency(match notification.urgency {
                Urgency::Low => notify_rust::Urgency::Low,
                Urgency::Normal => notify_rust::Urgency::Normal,
                Urgency::Critical => notify_rust::Urgency::Critical,
            });

            match builder.show() {
                Ok(_) => debug!(summary = %notification.summary, "notification shown"),
                Err(err) => warn!(%err, "could not show a notification"),
            }
        });
    }
}

/// Used when `notifications = false`.
pub struct SilentNotifier;

impl Notifier for SilentNotifier {
    fn notify(&self, notification: Notification) {
        debug!(summary = %notification.summary, "notification suppressed by config");
    }
}

pub fn for_config(config: &Config) -> Box<dyn Notifier> {
    if config.notifications {
        Box::new(DesktopNotifier)
    } else {
        Box::new(SilentNotifier)
    }
}

#[cfg(test)]
pub mod testing {
    use std::sync::{Arc, Mutex};

    use super::{Notification, Notifier};

    /// Captures notifications so tests can assert on what the user would have seen.
    #[derive(Clone, Default)]
    pub struct RecordingNotifier {
        sent: Arc<Mutex<Vec<Notification>>>,
    }

    impl RecordingNotifier {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn sent(&self) -> Vec<Notification> {
            self.sent.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    impl Notifier for RecordingNotifier {
        fn notify(&self, notification: Notification) {
            self.sent.lock().unwrap_or_else(|e| e.into_inner()).push(notification);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replaced(changes: u32) -> FixOutcome {
        FixOutcome::Replaced {
            text: "He goes.".into(),
            changes,
            model: "gpt-5.6-luna".into(),
            latency_ms: 42,
            cached: false,
        }
    }

    #[test]
    fn code_mode_puts_the_edit_count_in_the_body() {
        assert_eq!(notification_for(&replaced(1), Mode::Code).summary, "Code updated");
        let one = notification_for(&replaced(1), Mode::Code).body.unwrap();
        let three = notification_for(&replaced(3), Mode::Code).body.unwrap();
        assert_eq!(one, "1 edit pasted over your selection.");
        assert_eq!(three, "3 edits pasted over your selection.");
    }

    #[test]
    fn grammar_mode_counts_the_issues_in_the_summary() {
        // The wording each mode earned: a word diff means something on prose.
        assert_eq!(notification_for(&replaced(1), Mode::Grammar).summary, "Fixed 1 issue");
        assert_eq!(notification_for(&replaced(3), Mode::Grammar).summary, "Fixed 3 issues");
        assert_eq!(notification_for(&replaced(3), Mode::Grammar).body, None);
    }

    #[test]
    fn a_success_toast_is_low_urgency() {
        // A fix the user asked for shouldn't demand attention the way a failure does.
        assert_eq!(notification_for(&replaced(2), Mode::Code).urgency, Urgency::Low);
        assert_eq!(notification_for(&replaced(2), Mode::Grammar).urgency, Urgency::Low);
    }

    #[test]
    fn already_correct_says_so_without_alarming() {
        let outcome = FixOutcome::AlreadyCorrect {
            text: "Fine.".into(),
            model: "m".into(),
            latency_ms: 1,
            cached: true,
        };
        assert_eq!(notification_for(&outcome, Mode::Code).summary, "No changes needed");
        assert_eq!(notification_for(&outcome, Mode::Grammar).summary, "Looks good already");
        assert_eq!(notification_for(&outcome, Mode::Code).urgency, Urgency::Low);
    }

    #[test]
    fn no_selection_tells_the_user_what_to_do() {
        let code = notification_for(&FixOutcome::NoSelection, Mode::Code);
        assert_eq!(code.summary, "Nothing selected");
        assert!(code.body.unwrap().contains("Select some code"));

        let grammar = notification_for(&FixOutcome::NoSelection, Mode::Grammar);
        assert!(grammar.body.unwrap().contains("Select some text"));
    }

    #[test]
    fn failures_read_the_same_in_either_mode() {
        // A backend that is down is a backend that is down. Only the words for a
        // successful fix depend on what the fix was.
        let outcome = FixOutcome::Failed {
            code: "NO_API_KEY".into(),
            message: "Azure OpenAI is not configured on the backend.".into(),
            retryable: false,
        };
        assert_eq!(
            notification_for(&outcome, Mode::Code),
            notification_for(&outcome, Mode::Grammar)
        );
    }

    #[test]
    fn failures_are_critical_and_keep_the_detail() {
        let notification = notification_for(&FixOutcome::Failed {
            code: "NO_API_KEY".into(),
            message: "Azure OpenAI is not configured on the backend.".into(),
            retryable: false,
        }, Mode::Code);

        assert_eq!(notification.summary, "gramit backend has no API key");
        assert_eq!(notification.urgency, Urgency::Critical);
        assert!(notification.body.unwrap().contains("Azure OpenAI"));
    }

    #[test]
    fn known_codes_get_human_summaries() {
        assert_eq!(summary_for_code("BACKEND_UNREACHABLE"), "gramit backend is not running");
        assert_eq!(summary_for_code("TOO_LONG"), "Selection too long");
        assert_eq!(summary_for_code("INJECTION_ERROR"), "Could not paste the result");
    }

    #[test]
    fn an_unknown_code_still_produces_a_usable_toast() {
        let notification = notification_for(&FixOutcome::Failed {
            code: "SOMETHING_NEW".into(),
            message: "detail".into(),
            retryable: true,
        }, Mode::Code);
        assert_eq!(notification.summary, "Could not fix that");
        assert_eq!(notification.body.as_deref(), Some("detail"));
    }

    #[test]
    fn config_selects_the_notifier() {
        let mut config = Config { notifications: false, ..Config::default() };
        // Only the type differs; behaviour is covered by the recording notifier tests.
        let _silent = for_config(&config);
        config.notifications = true;
        let _desktop = for_config(&config);
    }
}
