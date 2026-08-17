//! Shared building blocks for the `gramit` CLI and the `gramitd` daemon:
//! configuration, the IPC protocol they speak over the local socket, and the
//! HTTP client for the backend.

pub mod client;
pub mod config;
pub mod error;
pub mod ipc;
pub mod paths;

pub use config::{default_backend_url, Capture, Config, Mode, DEFAULT_BACKEND_URL};
pub use error::{BackendError, ConfigError};

/// Version reported by `Ping` and `Status`, so a stale daemon is obvious.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
