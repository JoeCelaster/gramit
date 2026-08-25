use std::path::PathBuf;

use thiserror::Error;

/// A failure from the backend, or from trying to reach it.
///
/// `code` deliberately mirrors the backend's wire contract (`NO_API_KEY`,
/// `RATE_LIMITED`, …) and adds daemon-side codes for transport failures. The daemon
/// passes these straight through to the CLI and, later, to desktop notifications,
/// so every failure the user sees has a stable, greppable code.
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct BackendError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl BackendError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self { code: code.into(), message: message.into(), retryable }
    }

    /// The backend isn't listening — almost always "you forgot to start it".
    pub fn unreachable(url: &str, detail: impl std::fmt::Display) -> Self {
        Self::new(
            "BACKEND_UNREACHABLE",
            format!("Cannot reach the gramit backend at {url} ({detail}). Is it running?"),
            true,
        )
    }

    pub fn timeout(ms: u64) -> Self {
        Self::new("BACKEND_TIMEOUT", format!("The backend did not respond within {ms}ms."), true)
    }

    pub fn bad_response(detail: impl std::fmt::Display) -> Self {
        Self::new("BAD_RESPONSE", format!("The backend sent something unexpected: {detail}"), true)
    }

    pub fn too_long(length: usize, max: usize) -> Self {
        Self::new(
            "TOO_LONG",
            format!("Selection is {length} characters; the limit is {max}."),
            false,
        )
    }

    pub fn empty_text() -> Self {
        Self::new("EMPTY_TEXT", "No text to correct.", false)
    }

    /// No address has been configured yet. Deliberately distinct from `unreachable`:
    /// there is nothing to reach, and the remedy is a one-off setup step rather than
    /// waiting and retrying.
    pub fn not_configured() -> Self {
        Self::new("NO_BACKEND", "No backend is configured. Set one with: gramit setup", false)
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },

    #[error("could not write {path}: {source}")]
    Write { path: PathBuf, source: std::io::Error },

    #[error("{path} is not valid config: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("could not serialize config: {0}")]
    Serialize(String),

    #[error("invalid config: {0}")]
    Invalid(String),

    #[error("could not determine the config directory for this platform")]
    NoConfigDir,
}
