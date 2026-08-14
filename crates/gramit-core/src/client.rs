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

impl BackendClient {
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, BackendError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
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
        if err.is_timeout() {
            BackendError::timeout(self.timeout_ms)
        } else if err.is_connect() {
            BackendError::unreachable(&self.base_url, err)
        } else if err.is_decode() {
            BackendError::bad_response(err)
        } else {
            BackendError::unreachable(&self.base_url, err)
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
        let body = FixRequestBody { text: "he go", mode: Mode::Grammar };
        assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"text":"he go","mode":"grammar"}"#);
    }
}
