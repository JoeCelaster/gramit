//! Test doubles for the platform traits, so the daemon's fix loop can be tested
//! without a desktop session.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::clipboard::Clipboard;
use crate::injector::Injector;
use crate::InputError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Copy,
    Paste,
}

#[derive(Default)]
struct Shared {
    contents: Option<String>,
    actions: Vec<Action>,
    /// What a simulated Ctrl+C puts on the clipboard. `None` models "nothing
    /// selected", where the clipboard is left untouched.
    selection: Option<String>,
    fail_copy: Option<String>,
    fail_paste: Option<String>,
    fail_set: Option<String>,
    /// The user's PRIMARY selection, published without any copy.
    primary: Option<String>,
    /// Copies before this many attempts do nothing, modelling a hotkey still held
    /// down so the injected Ctrl+C arrives as Ctrl+Alt+C.
    copy_succeeds_on_attempt: u32,
    copy_attempts: u32,
}

/// A clipboard and injector pair sharing one state, so a fake `copy()` lands text on
/// the fake clipboard exactly as the real pair would.
#[derive(Clone, Default)]
pub struct FakeDesktop {
    shared: Arc<Mutex<Shared>>,
}

impl FakeDesktop {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets what the user has highlighted, which a `copy()` will pick up.
    pub fn with_selection(self, selection: &str) -> Self {
        self.lock().selection = Some(selection.to_string());
        self
    }

    pub fn with_clipboard(self, contents: &str) -> Self {
        self.lock().contents = Some(contents.to_string());
        self
    }

    /// Makes the first `n - 1` copies silently do nothing.
    pub fn copy_succeeds_on_attempt(self, n: u32) -> Self {
        self.lock().copy_succeeds_on_attempt = n;
        self
    }

    pub fn with_primary(self, primary: &str) -> Self {
        self.lock().primary = Some(primary.to_string());
        self
    }

    pub fn copy_attempts(&self) -> u32 {
        self.lock().copy_attempts
    }

    pub fn failing_copy(self, message: &str) -> Self {
        self.lock().fail_copy = Some(message.to_string());
        self
    }

    pub fn failing_paste(self, message: &str) -> Self {
        self.lock().fail_paste = Some(message.to_string());
        self
    }

    pub fn failing_set_text(self, message: &str) -> Self {
        self.lock().fail_set = Some(message.to_string());
        self
    }

    pub fn clipboard_contents(&self) -> Option<String> {
        self.lock().contents.clone()
    }

    pub fn actions(&self) -> Vec<Action> {
        self.lock().actions.clone()
    }

    pub fn clipboard(&self) -> FakeClipboard {
        FakeClipboard { shared: Arc::clone(&self.shared) }
    }

    pub fn injector(&self) -> FakeInjector {
        FakeInjector { shared: Arc::clone(&self.shared) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct FakeClipboard {
    shared: Arc<Mutex<Shared>>,
}

impl FakeClipboard {
    pub fn empty() -> Self {
        FakeDesktop::new().clipboard()
    }

    pub fn with_text(text: &str) -> Self {
        FakeDesktop::new().with_clipboard(text).clipboard()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl Clipboard for FakeClipboard {
    async fn get_text(&self) -> Result<Option<String>, InputError> {
        Ok(self.lock().contents.clone())
    }

    async fn set_text(&self, text: String) -> Result<(), InputError> {
        let mut shared = self.lock();
        if let Some(message) = &shared.fail_set {
            return Err(InputError::Clipboard(message.clone()));
        }
        shared.contents = Some(text);
        Ok(())
    }

    async fn clear(&self) -> Result<(), InputError> {
        self.lock().contents = None;
        Ok(())
    }

    async fn get_primary_text(&self) -> Result<Option<String>, InputError> {
        Ok(self.lock().primary.clone())
    }
}

pub struct FakeInjector {
    shared: Arc<Mutex<Shared>>,
}

impl FakeInjector {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl Injector for FakeInjector {
    async fn copy(&self) -> Result<(), InputError> {
        let mut shared = self.lock();
        shared.actions.push(Action::Copy);
        shared.copy_attempts += 1;

        if let Some(message) = &shared.fail_copy {
            return Err(InputError::Injection(message.clone()));
        }

        // Modelling a held modifier: the keystroke is delivered but the app sees the
        // wrong chord, so nothing reaches the clipboard and no error is raised.
        if shared.copy_attempts < shared.copy_succeeds_on_attempt {
            return Ok(());
        }

        // Nothing selected leaves the clipboard alone, exactly like a real Ctrl+C.
        if let Some(selection) = shared.selection.clone() {
            shared.contents = Some(selection);
        }
        Ok(())
    }

    async fn paste(&self) -> Result<(), InputError> {
        let mut shared = self.lock();
        shared.actions.push(Action::Paste);
        if let Some(message) = &shared.fail_paste {
            return Err(InputError::Injection(message.clone()));
        }
        Ok(())
    }

    fn describe(&self) -> String {
        "fake injector".to_string()
    }
}
