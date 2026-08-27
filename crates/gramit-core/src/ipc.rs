//! The line protocol between `gramit` (CLI) and `gramitd` (daemon).
//!
//! One JSON object per line, request and response. Newline-delimited JSON keeps the
//! wire debuggable by hand (`nc -U`) and lets a single connection carry several
//! requests without any framing header.

use serde::{Deserialize, Serialize};

use crate::config::Mode;
use crate::error::BackendError;

/// Longest line either side will accept, so a runaway peer can't exhaust memory.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Liveness and version check.
    Ping,
    /// Correct the supplied text and return it; nothing is pasted anywhere.
    Fix {
        text: String,
        /// `None` means "whatever mode the daemon is in", which is what `gramit fix`
        /// sends unless `--mode` overrides it. The daemon's config is the authority:
        /// having the CLI read the mode itself would let the two disagree whenever
        /// the file changed after the daemon started.
        #[serde(default)]
        mode: Option<Mode>,
    },
    /// Run the full capture → correct → paste loop against the current selection.
    FixSelection,
    /// Correct whatever is on the clipboard, in place.
    ///
    /// The daemon does this rather than the CLI because on X11 the process that sets
    /// the clipboard must stay alive to serve it — a CLI that exited immediately
    /// would take the corrected text with it.
    FixClipboard,
    Status,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Pong {
        version: String,
        pid: u32,
        uptime_s: u64,
    },
    Fixed {
        text: String,
        changed: bool,
        changes: u32,
        model: String,
        latency_ms: u64,
        cached: bool,
    },
    Status(Box<StatusReport>),
    /// Acknowledgement for requests with no payload (currently `Shutdown`).
    Ok,
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
}

impl Response {
    pub fn error(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Response::Error { code: code.into(), message: message.into(), retryable }
    }
}

impl From<BackendError> for Response {
    fn from(err: BackendError) -> Self {
        Response::Error { code: err.code, message: err.message, retryable: err.retryable }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    pub version: String,
    pub pid: u32,
    pub uptime_s: u64,
    pub hotkey: String,
    /// Whether the OS accepted our global hotkey registration.
    pub hotkey_registered: bool,
    /// What the OS actually bound, which can differ from `hotkey`.
    pub hotkey_detail: Option<String>,
    /// Whether clipboard + keystroke injection are both available, i.e. whether
    /// `FixSelection` can work at all.
    pub selection_ready: bool,
    /// The injection mechanism in use, e.g. "RemoteDesktop portal (Wayland)".
    pub injector: Option<String>,
    /// `None` when no backend has been configured yet.
    pub backend_url: Option<String>,
    pub backend_reachable: bool,
    /// Whether the backend reports a usable Azure configuration.
    pub backend_has_key: Option<bool>,
    pub backend_detail: Option<String>,
    pub mode: Mode,
    pub notifications: bool,
    pub fixes_total: u64,
    pub last_error: Option<String>,
}

/// Serializes a message as one protocol line, newline included.
pub fn encode<T: Serialize>(message: &T) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

pub fn decode_request(line: &str) -> Result<Request, serde_json::Error> {
    serde_json::from_str(line)
}

pub fn decode_response(line: &str) -> Result<Response, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_request(request: Request) {
        let line = encode(&request).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(decode_request(line.trim_end()).unwrap(), request);
    }

    #[test]
    fn requests_round_trip() {
        round_trip_request(Request::Ping);
        round_trip_request(Request::Status);
        round_trip_request(Request::Shutdown);
        round_trip_request(Request::FixSelection);
        round_trip_request(Request::FixClipboard);
        round_trip_request(Request::Fix { text: "he go".into(), mode: Some(Mode::Grammar) });
        round_trip_request(Request::Fix { text: "f()".into(), mode: None });
    }

    #[test]
    fn fix_without_a_mode_defers_to_the_daemon() {
        let request = decode_request(r#"{"type":"fix","text":"hello"}"#).unwrap();
        assert_eq!(request, Request::Fix { text: "hello".into(), mode: None });
    }

    #[test]
    fn requests_are_tagged_by_type() {
        assert_eq!(encode(&Request::Ping).unwrap().trim_end(), r#"{"type":"ping"}"#);
    }

    #[test]
    fn responses_round_trip() {
        let response = Response::Fixed {
            text: "He goes.".into(),
            changed: true,
            changes: 2,
            model: "gpt-5.6-luna".into(),
            latency_ms: 340,
            cached: false,
        };
        let line = encode(&response).unwrap();
        assert_eq!(decode_response(line.trim_end()).unwrap(), response);
    }

    #[test]
    fn status_round_trips() {
        let report = StatusReport {
            version: "0.1.0".into(),
            pid: 42,
            uptime_s: 7,
            hotkey: "Ctrl+Alt+F".into(),
            hotkey_registered: false,
            hotkey_detail: None,
            selection_ready: true,
            injector: Some("fake".into()),
            backend_url: Some("http://127.0.0.1:8787".into()),
            backend_reachable: true,
            backend_has_key: Some(false),
            backend_detail: Some("missing AZURE_OPENAI_API_KEY".into()),
            mode: Mode::Code,
            notifications: true,
            fixes_total: 3,
            last_error: None,
        };
        let response = Response::Status(Box::new(report));
        let line = encode(&response).unwrap();
        assert_eq!(decode_response(line.trim_end()).unwrap(), response);
    }

    #[test]
    fn backend_errors_become_error_responses() {
        let err = BackendError::new("NO_API_KEY", "not configured", false);
        match Response::from(err) {
            Response::Error { code, retryable, .. } => {
                assert_eq!(code, "NO_API_KEY");
                assert!(!retryable);
            }
            other => panic!("expected an error response, got {other:?}"),
        }
    }

    #[test]
    fn unknown_request_type_is_rejected() {
        assert!(decode_request(r#"{"type":"selfdestruct"}"#).is_err());
        assert!(decode_request("not json").is_err());
    }
}
