//! Platform input for gramit: the clipboard, the global hotkey, and synthetic
//! copy/paste keystrokes.
//!
//! Everything above this crate works against the three traits here, so the daemon's
//! fix loop is testable with fakes and identical on every OS. The platform-specific
//! parts live behind [`clipboard::open`], [`hotkey::register`], and
//! [`injector::open`], which pick an implementation for the current OS.

pub mod clipboard;
pub mod fake;
pub mod hotkey;
pub mod hotkey_spec;
pub mod injector;
pub mod run_loop;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod native;

/// GNOME custom-keybinding management, used when the GlobalShortcuts portal refuses
/// to bind a hotkey for us. `gramit doctor` drives this in Module 3.
#[cfg(target_os = "linux")]
pub use linux::gnome as linux_gnome;

pub use clipboard::{Clipboard, ClipboardSnapshot};
pub use hotkey::{HotkeyEvent, HotkeyRegistration};
pub use injector::Injector;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("clipboard: {0}")]
    Clipboard(String),

    #[error("hotkey: {0}")]
    Hotkey(String),

    #[error("keystroke injection: {0}")]
    Injection(String),

    #[error("desktop portal: {0}")]
    Portal(String),

    #[error("{0}")]
    Unsupported(String),
}

impl InputError {
    /// Stable code for notifications and `gramit doctor`, matching the daemon's
    /// convention that every user-visible failure carries a greppable code.
    pub fn code(&self) -> &'static str {
        match self {
            InputError::Clipboard(_) => "CLIPBOARD_ERROR",
            InputError::Hotkey(_) => "HOTKEY_ERROR",
            InputError::Injection(_) => "INJECTION_ERROR",
            InputError::Portal(_) => "PORTAL_ERROR",
            InputError::Unsupported(_) => "UNSUPPORTED_PLATFORM",
        }
    }
}
