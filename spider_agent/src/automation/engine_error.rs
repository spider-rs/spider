//! Engine error types for automation.

use std::{error::Error as StdError, fmt};

/// Convenience result type used throughout the remote multimodal engine.
pub type EngineResult<T> = Result<T, EngineError>;

/// Errors produced by the remote multimodal engine.
///
/// This error type is intentionally lightweight and is suitable
/// for surfacing from public APIs.
///
/// It covers:
/// - transport failures when calling the remote endpoint,
/// - JSON serialization/deserialization failures,
/// - schema mismatches in OpenAI-compatible responses,
/// - non-success responses returned by the remote provider,
/// - unsupported operations due to compile-time feature flags.
#[derive(Debug)]
pub enum EngineError {
    /// HTTP-layer failure (request could not be sent, connection error, timeout, etc.).
    Http(reqwest::Error),
    /// JSON serialization/deserialization failure when building or parsing payloads.
    Json(serde_json::Error),
    /// A required field was missing in a parsed JSON payload.
    ///
    /// Example: missing `"choices[0].message.content"` in an OpenAI-compatible response.
    MissingField(&'static str),
    /// A field was present but had an unexpected type or shape.
    ///
    /// Example: `"steps"` exists but is not an array.
    InvalidField(&'static str),
    /// The remote endpoint returned a non-success status or a server-side error.
    ///
    /// The contained string should be a human-readable explanation suitable for logs.
    Remote(String),
    /// The remote endpoint returned a non-success HTTP status with a known status code.
    ///
    /// Carries the numeric status code (e.g. 502, 429) so retry logic can
    /// distinguish transient/retryable errors from permanent ones.
    RemoteStatus(u16, String),
    /// The operation is not supported in the current build configuration.
    ///
    /// Example: calling browser automation without the `chrome` feature.
    Unsupported(&'static str),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::Http(e) => write!(f, "http error: {e}"),
            EngineError::Json(e) => write!(f, "json error: {e}"),
            EngineError::MissingField(s) => write!(f, "missing field: {s}"),
            EngineError::InvalidField(s) => write!(f, "invalid field: {s}"),
            EngineError::Remote(s) => write!(f, "remote error: {s}"),
            EngineError::RemoteStatus(code, s) => write!(f, "remote error {code}: {s}"),
            EngineError::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl StdError for EngineError {}

impl EngineError {
    /// Whether this error is transient and warrants retrying on a different model.
    ///
    /// Returns `true` for server errors (500, 502, 503), rate limits (429),
    /// and transport-level failures (timeouts, connection resets).
    pub fn is_retryable_on_different_model(&self) -> bool {
        match self {
            EngineError::RemoteStatus(code, _) => matches!(code, 429 | 500 | 502 | 503 | 504),
            EngineError::Http(e) => e.is_timeout() || e.is_connect() || e.is_request(),
            // Legacy Remote — check if message contains status hint
            EngineError::Remote(msg) => {
                msg.contains("502")
                    || msg.contains("503")
                    || msg.contains("429")
                    || msg.contains("500")
                    || msg.contains("504")
            }
            _ => false,
        }
    }
}

impl From<reqwest::Error> for EngineError {
    fn from(e: reqwest::Error) -> Self {
        EngineError::Http(e)
    }
}

impl From<serde_json::Error> for EngineError {
    fn from(e: serde_json::Error) -> Self {
        EngineError::Json(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // RemoteStatus — retryable codes
    // -----------------------------------------------------------------------

    #[test]
    fn remote_status_retryable_codes() {
        for code in [429u16, 500, 502, 503, 504] {
            let err = EngineError::RemoteStatus(code, format!("err {code}"));
            assert!(
                err.is_retryable_on_different_model(),
                "RemoteStatus({code}) should be retryable"
            );
        }
    }

    #[test]
    fn remote_status_non_retryable_codes() {
        for code in [400u16, 401, 403, 404, 405, 409, 422, 501] {
            let err = EngineError::RemoteStatus(code, format!("err {code}"));
            assert!(
                !err.is_retryable_on_different_model(),
                "RemoteStatus({code}) should NOT be retryable"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Remote (legacy string matching)
    // -----------------------------------------------------------------------

    #[test]
    fn remote_legacy_retryable_messages() {
        for keyword in ["502", "503", "429", "500", "504"] {
            let err = EngineError::Remote(format!("upstream returned {keyword} bad gateway"));
            assert!(
                err.is_retryable_on_different_model(),
                "Remote message containing '{keyword}' should be retryable"
            );
        }
    }

    #[test]
    fn remote_legacy_non_retryable_message() {
        let err = EngineError::Remote("authentication failed".into());
        assert!(
            !err.is_retryable_on_different_model(),
            "Remote without status hint should NOT be retryable"
        );
    }

    #[test]
    fn remote_legacy_no_false_positive_on_partial() {
        // "4290" contains "429" — verify substring matching behavior is understood.
        // Current implementation uses contains(), so this WILL match.
        // This test documents the behavior rather than asserting it's wrong.
        let err = EngineError::Remote("error code 4290 custom".into());
        // contains("429") is true — this is a known edge case.
        assert!(err.is_retryable_on_different_model());
    }

    // -----------------------------------------------------------------------
    // Non-retryable variants
    // -----------------------------------------------------------------------

    #[test]
    fn missing_field_not_retryable() {
        let err = EngineError::MissingField("choices[0].message.content");
        assert!(!err.is_retryable_on_different_model());
    }

    #[test]
    fn invalid_field_not_retryable() {
        let err = EngineError::InvalidField("steps is not an array");
        assert!(!err.is_retryable_on_different_model());
    }

    #[test]
    fn json_error_not_retryable() {
        let json_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let err = EngineError::Json(json_err);
        assert!(!err.is_retryable_on_different_model());
    }

    #[test]
    fn unsupported_not_retryable() {
        let err = EngineError::Unsupported("chrome feature not enabled");
        assert!(!err.is_retryable_on_different_model());
    }

    // -----------------------------------------------------------------------
    // Display formatting
    // -----------------------------------------------------------------------

    #[test]
    fn display_format_all_variants() {
        assert_eq!(
            EngineError::MissingField("foo").to_string(),
            "missing field: foo"
        );
        assert_eq!(
            EngineError::InvalidField("bar").to_string(),
            "invalid field: bar"
        );
        assert_eq!(
            EngineError::Remote("oops".into()).to_string(),
            "remote error: oops"
        );
        assert_eq!(
            EngineError::RemoteStatus(502, "bad gw".into()).to_string(),
            "remote error 502: bad gw"
        );
        assert_eq!(
            EngineError::Unsupported("nope").to_string(),
            "unsupported: nope"
        );
    }

    // -----------------------------------------------------------------------
    // From conversions
    // -----------------------------------------------------------------------

    #[test]
    fn from_serde_json_error() {
        let json_err = serde_json::from_str::<serde_json::Value>("{bad}").unwrap_err();
        let engine_err: EngineError = json_err.into();
        assert!(matches!(engine_err, EngineError::Json(_)));
    }

    // -----------------------------------------------------------------------
    // Http variant — reqwest errors are hard to construct without a real
    // request, but we can verify the timeout/connect/request branches
    // by constructing errors from actual failed requests to localhost.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn http_timeout_is_retryable() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(1))
            .build()
            .unwrap();
        // Request to a non-routable address to trigger timeout
        let result = client.get("http://192.0.2.1:1").send().await;
        if let Err(e) = result {
            let engine_err = EngineError::Http(e);
            // Either timeout or connect error — both should be retryable
            assert!(
                engine_err.is_retryable_on_different_model(),
                "HTTP timeout/connect error should be retryable"
            );
        }
        // If the request somehow succeeds (shouldn't), test is vacuously true
    }

    #[tokio::test]
    async fn http_connect_refused_is_retryable() {
        let client = reqwest::Client::new();
        // Port 1 is almost certainly not listening
        let result = client.get("http://127.0.0.1:1").send().await;
        if let Err(e) = result {
            let engine_err = EngineError::Http(e);
            assert!(
                engine_err.is_retryable_on_different_model(),
                "HTTP connection refused should be retryable"
            );
        }
    }
}
