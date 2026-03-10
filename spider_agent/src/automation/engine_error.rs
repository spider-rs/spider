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
