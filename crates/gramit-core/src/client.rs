use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::Mode;
use crate::error::BackendError;

/// Mirrors the backend's `POST /v1/fix` response body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixOutcome {
    pub corrected: String,
    pub changed: bool,
    pub changes: u32,
    pub model: String,
    pub latency_ms: u64,
    #[serde(default)]
    pub cached: bool,
}

/// Mirrors the backend's `GET /health` response body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub ok: bool,
    #[serde(default)]
    pub version: String,
    #[serde(rename = "hasKey")]
    pub has_key: bool,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub missing: Vec<String>,
}

/// The backend's error envelope: `{"error": {"code", "message", "retryable"}}`.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
    #[serde(default)]
    retryable: bool,
}

#[derive(Debug, Serialize)]
struct FixRequestBody<'a> {
    text: &'a str,
    mode: Mode,
}

#[derive(Debug, Clone)]
pub struct BackendClient {
    http: reqwest::Client,
    base_url: String,
    timeout_ms: u64,
}

/// How the transport failed, in the terms the user's notification is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Unreachable,
    Timeout,
    BadResponse,
}

/// Which of the three a reqwest error is.
///
/// `is_connect` is checked before `is_timeout` because a connect that times out is
/// both, and "the backend is not running" is the more useful of the two things to tell
/// someone: they can start it. A timeout means the backend answered the phone and then
/// took too long, which is a different problem with a different remedy.
fn classify_transport(is_connect: bool, is_timeout: bool, is_decode: bool) -> Transport {
    if is_connect {
        Transport::Unreachable
    } else if is_timeout {
        Transport::Timeout
    } else if is_decode {
        Transport::BadResponse
    } else {
        Transport::Unreachable
    }
}

impl BackendClient {
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, BackendError> {
        // A separate, shorter budget for getting the connection open at all. Without
        // one, a host that never answers — a stopped backend on Windows, which drops
        // the SYN instead of refusing it — spends the whole request timeout and is
        // then reported as "timed out" rather than "not running". Half the budget, so
        // it always fires first, capped so a generous request timeout does not mean
        // waiting a minute to learn nothing is listening.
        let connect_timeout = (timeout / 2).min(Duration::from_secs(5));

        let http = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(connect_timeout)
            .build()
            .map_err(|err| BackendError::new("CLIENT_INIT", format!("HTTP client: {err}"), false))?;

        Ok(Self {
            http,
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            timeout_ms: timeout.as_millis() as u64,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn health(&self) -> Result<Health, BackendError> {
        let response = self
            .http
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .map_err(|err| self.transport_error(err))?;

        if !response.status().is_success() {
            return Err(self.error_from_response(response).await);
        }

        response.json::<Health>().await.map_err(BackendError::bad_response)
    }

    pub async fn fix(&self, text: &str, mode: Mode) -> Result<FixOutcome, BackendError> {
        let response = self
            .http
            .post(format!("{}/v1/fix", self.base_url))
            .json(&FixRequestBody { text, mode })
            .send()
            .await
            .map_err(|err| self.transport_error(err))?;

        if !response.status().is_success() {
            return Err(self.error_from_response(response).await);
        }

        response.json::<FixOutcome>().await.map_err(BackendError::bad_response)
    }

    fn transport_error(&self, err: reqwest::Error) -> BackendError {
        match classify_transport(err.is_connect(), err.is_timeout(), err.is_decode()) {
            Transport::Unreachable => BackendError::unreachable(&self.base_url, err),
            Transport::Timeout => BackendError::timeout(self.timeout_ms),
            Transport::BadResponse => BackendError::bad_response(err),
        }
    }

    /// Prefers the backend's own `code`, so `NO_API_KEY` survives all the way to the
    /// user's notification instead of collapsing into a generic HTTP 503.
    async fn error_from_response(&self, response: reqwest::Response) -> BackendError {
        let status = response.status();
        match response.json::<ErrorEnvelope>().await {
            Ok(envelope) => BackendError::new(
                envelope.error.code,
                envelope.error.message,
                envelope.error.retryable,
            ),
            Err(_) => BackendError::new(
                "BAD_RESPONSE",
                format!("The backend returned HTTP {status} with no error details."),
                status.is_server_error(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_trailing_slash_from_the_base_url() {
        let client = BackendClient::new("http://127.0.0.1:8787/", Duration::from_secs(1)).unwrap();
        assert_eq!(client.base_url(), "http://127.0.0.1:8787");
    }

    #[test]
    fn deserializes_a_fix_outcome() {
        let json = r#"{"corrected":"He goes.","changed":true,"changes":2,
                       "model":"gpt-5.6-luna","latency_ms":340,"cached":false}"#;
        let outcome: FixOutcome = serde_json::from_str(json).unwrap();
        assert_eq!(outcome.corrected, "He goes.");
        assert_eq!(outcome.changes, 2);
        assert!(!outcome.cached);
    }

    #[test]
    fn deserializes_health_with_camel_case_has_key() {
        let json = r#"{"ok":true,"version":"0.1.0","hasKey":false,"model":null,
                       "missing":["AZURE_OPENAI_API_KEY"]}"#;
        let health: Health = serde_json::from_str(json).unwrap();
        assert!(!health.has_key);
        assert_eq!(health.missing, vec!["AZURE_OPENAI_API_KEY"]);
    }

    #[test]
    fn deserializes_the_error_envelope() {
        let json = r#"{"error":{"code":"NO_API_KEY","message":"not configured","retryable":false}}"#;
        let envelope: ErrorEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.error.code, "NO_API_KEY");
    }

    #[test]
    fn serializes_the_fix_request_body() {
        let body = FixRequestBody { text: "he go", mode: Mode::Code };
        assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"text":"he go","mode":"code"}"#);
    }

    #[test]
    fn a_connect_that_times_out_is_unreachable_not_a_timeout() {
        // The Windows CI failure this exists to prevent. A refused connection (Linux,
        // macOS) is a plain connect error; a dropped SYN (Windows, and any firewall
        // that blackholes) is a connect error *and* a timeout, because the connect
        // budget is what ran out. Both mean the same thing to the user: nothing is
        // listening. Reading them differently made the same test pass on one platform
        // and fail on another.
        assert_eq!(classify_transport(true, false, false), Transport::Unreachable);
        assert_eq!(classify_transport(true, true, false), Transport::Unreachable);
    }

    #[test]
    fn a_slow_backend_is_still_a_timeout() {
        // Connected, then took too long: a different problem with a different remedy,
        // so it must not be swallowed by the rule above.
        assert_eq!(classify_transport(false, true, false), Transport::Timeout);
    }

    #[test]
    fn an_unreadable_body_and_an_unknown_failure_keep_their_meanings() {
        assert_eq!(classify_transport(false, false, true), Transport::BadResponse);
        // Nothing else to go on: "unreachable" is the honest, actionable guess.
        assert_eq!(classify_transport(false, false, false), Transport::Unreachable);
    }

    #[test]
    fn the_connect_budget_always_expires_before_the_request_budget() {
        // If they were equal the two could race, and the classification above would
        // come down to which timer the OS fired first.
        for ms in [200_u64, 500, 15_000, 60_000] {
            let timeout = Duration::from_millis(ms);
            let connect = (timeout / 2).min(Duration::from_secs(5));
            assert!(connect < timeout, "connect budget must be shorter for {ms}ms");
        }
    }

    #[test]
    fn every_mode_goes_on_the_wire_as_the_backend_spells_it() {
        // The backend rejects any mode outside its own list, so a name that differs by
        // a letter is a 400 on every fix in that mode.
        for (mode, expected) in [
            (Mode::Code, "code"),
            (Mode::Grammar, "grammar"),
            (Mode::Write, "write"),
            (Mode::Prompt, "prompt"),
        ] {
            let body = FixRequestBody { text: "x", mode };
            let json = serde_json::to_string(&body).unwrap();
            assert_eq!(json, format!(r#"{{"text":"x","mode":"{expected}"}}"#));
        }
    }
}


