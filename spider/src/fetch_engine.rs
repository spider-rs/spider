//! Pluggable in-process HTTP fetch engine for [`crate::website::Website`].
//!
//! Implementing [`HttpFetchEngine`] and installing it on a
//! [`Website`](crate::website::Website) via
//! [`with_fetch_engine`](crate::website::Website::with_fetch_engine)
//! reroutes spider's per-URL HTTP body fetch through the user's code
//! **while keeping every surrounding spider concern in spider's hands** —
//! the first-byte watchdog, truncation retry, TLS scheme-flip retry,
//! NXDOMAIN short-circuit, cache layers, hedged requests, link
//! extraction, visited tracking, and [`crate::page::build`] status
//! reclassification all still wrap the engine call.
//!
//! ## How it differs from [`crate::fetcher::RemoteFetcher`]
//!
//! [`RemoteFetcher`](crate::fetcher::RemoteFetcher) short-circuits the
//! *entire* crawl loop before spider's fetch machinery runs — the
//! implementor owns retries, caching, hedging, everything.
//!
//! [`HttpFetchEngine`] is the opposite: it replaces only the innermost
//! "send the request and read the bytes" step, so spider's machinery
//! stays wrapped around it. Use this when you want a different transport
//! for the raw HTTP body but want to keep spider's retry/cache/watchdog
//! behavior byte-for-byte.
//!
//! ## Default behavior unchanged
//!
//! A `Website` with **no** engine installed (the default) runs the exact
//! same reqwest fetch path it always has. The hook is purely additive and
//! is only compiled in under the `fetch_engine` cargo feature; with the
//! feature off there is no engine field, no branch, and no cost.
//!
//! ## Per-URL opt-in
//!
//! [`HttpFetchEngine::should_fetch`] is consulted per URL before the
//! engine is used. Returning `false` makes spider fall through to its
//! built-in reqwest fetch for that URL. Any rollout ramp (percentage
//! gating, per-domain breakers, kill switches) lives inside the
//! implementor — spider stays transport- and policy-neutral.
//!
//! ## Scope (today)
//!
//! The engine fires on the **HTTP** crawl paths: the raw streaming path
//! ([`Website::crawl`](crate::website::Website::crawl) /
//! [`crawl_raw`](crate::website::Website::crawl_raw)) and the HTTP-first
//! attempt of the smart path
//! ([`crawl_smart`](crate::website::Website::crawl_smart)). Browser
//! navigation (chrome / webdriver, and the smart path's chrome upgrade)
//! is never routed through the engine.

use std::sync::Arc;

use crate::client::StatusCode;
use crate::configuration::Configuration;

/// HTTP method the engine is asked to perform. Spider's crawl fetch is
/// always `GET`; the enum leaves room without widening the trait later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EngineMethod {
    /// HTTP GET.
    #[default]
    Get,
}

/// Per-request context handed to an [`HttpFetchEngine`]. Borrowed for the
/// duration of one fetch call.
///
/// The engine builds and owns its own client, so this carries the raw
/// crawl configuration (proxies, headers, user agent, timeouts, redirect
/// limit, cookie string, cert policy, HTTP/2 knob) it needs to match what
/// spider's reqwest client would have used. Nothing forces the engine to
/// consult any given field.
#[derive(Debug)]
pub struct EngineRequest<'a> {
    /// The target URL. May be scheme-flipped (`http`↔`https`) or
    /// `www`-stripped on a TLS-handshake retry, so honor it verbatim.
    pub url: &'a str,
    /// The HTTP method (always [`EngineMethod::Get`] today).
    pub method: EngineMethod,
    /// The crawl-level configuration for parity with the reqwest path.
    pub configuration: &'a Configuration,
    /// When `true`, spider only wants HTML — the engine may early-out on
    /// clearly-binary bodies. Mirrors the reqwest path's `only_html`.
    pub only_html: bool,
    /// Retry attempt counter (0 = first try). A rotation / failover hint.
    pub attempt: u32,
    /// Conditional-request validators (ETag / Last-Modified) to send, when
    /// spider is doing a conditional fetch. `None` for a plain fetch.
    pub conditional_headers: Option<&'a [(String, String)]>,
}

/// A completed engine response. The engine returns a fully-decoded body
/// (decompressed, charset-normalized on its side); spider then applies
/// the same size caps, UTF-8 validation, anti-bot detection, and status
/// reclassification it applies to a reqwest response, via
/// [`crate::utils::handle_engine_response_bytes`].
#[derive(Debug, Default)]
pub struct EngineResponse {
    /// The response status code.
    pub status_code: StatusCode,
    /// The final URL after any redirects (set only when it differs from
    /// the requested URL, matching the reqwest path's redirect detection).
    pub final_url: Option<String>,
    /// The response headers.
    pub headers: reqwest::header::HeaderMap,
    /// The peer socket address, when the engine can surface it.
    #[cfg(feature = "remote_addr")]
    pub remote_addr: Option<core::net::SocketAddr>,
    /// `Set-Cookie` values parsed the same way spider parses them from a
    /// reqwest response, so downstream cookie handling sees parity.
    #[cfg(feature = "cookies")]
    pub response_cookies: Option<reqwest::header::HeaderMap>,
    /// The fully-decoded response body.
    pub body: Vec<u8>,
    /// The declared `Content-Length`, when known, so spider can detect a
    /// truncated body (fewer bytes received than promised).
    pub declared_content_length: Option<u64>,
    /// Optional anti-bot / WAF classification the engine already computed.
    /// When `None`, spider runs its own body-based detection.
    pub anti_bot_tech: Option<crate::page::AntiBotTech>,
    /// Whether the engine actually served this response (vs. a
    /// pass-through / synthetic result). Consumers can read this to
    /// attribute traffic; spider itself does not branch on it.
    pub served: bool,
}

/// Transport failure categories. These map onto the same synthetic status
/// codes and retry decisions spider derives from a `reqwest::Error`, so
/// the engine path keeps identical retry semantics without spider having
/// to inspect a concrete error type.
#[derive(Debug, Clone)]
pub enum EngineError {
    /// Request timed out (connect / read / overall). → `524`.
    Timeout,
    /// TLS handshake failure. → `526`, and triggers the scheme-flip /
    /// strip-`www` retry ladder just like the reqwest path.
    TlsHandshake,
    /// DNS resolution failed. → `525`.
    Dns,
    /// Connection refused by the origin. → `521`.
    ConnectRefused,
    /// Connection aborted. → `522`.
    ConnectAborted,
    /// Connection reset. → `523`.
    ConnectReset,
    /// Host/network permanently unreachable. → `526`.
    AddressUnreachable,
    /// Proxy CONNECT / tunnel failure. → `503`, then upgraded to `525`
    /// if an independent local DNS lookup confirms NXDOMAIN.
    ProxyTunnel,
    /// Response body decode failure. → `400`.
    Body,
    /// Malformed request. → `400 BAD_REQUEST`.
    Request,
    /// A real upstream status the engine wants to surface as an error.
    Status(u16),
    /// Anything else. → `599`.
    Other(String),
}

impl EngineError {
    /// Map the error category onto the synthetic status code spider uses
    /// for the equivalent reqwest failure. Kept in lockstep with the
    /// `*_ERROR` status constants in [`crate::page`].
    pub fn to_status_code(&self) -> StatusCode {
        match self {
            EngineError::Timeout => *crate::page::CONNECTION_TIMEOUT_ERROR,
            EngineError::TlsHandshake => *crate::page::ADDRESS_UNREACHABLE_ERROR,
            EngineError::Dns => *crate::page::DNS_RESOLVE_ERROR,
            EngineError::ConnectRefused => *crate::page::CONNECTION_REFUSED_ERROR,
            EngineError::ConnectAborted => *crate::page::CONNECTION_ABORTED_ERROR,
            EngineError::ConnectReset => *crate::page::CONNECTION_RESET_ERROR,
            EngineError::AddressUnreachable => *crate::page::ADDRESS_UNREACHABLE_ERROR,
            EngineError::ProxyTunnel => *crate::page::UNREACHABLE_REQUEST_ERROR,
            EngineError::Body => *crate::page::BODY_DECODE_ERROR,
            EngineError::Request => StatusCode::BAD_REQUEST,
            EngineError::Status(code) => {
                StatusCode::from_u16(*code).unwrap_or(*crate::page::UNKNOWN_STATUS_ERROR)
            }
            EngineError::Other(_) => *crate::page::UNKNOWN_STATUS_ERROR,
        }
    }

    /// Whether this failure should trigger spider's TLS scheme-flip /
    /// strip-`www` retry ladder.
    pub fn is_handshake_failure(&self) -> bool {
        matches!(self, EngineError::TlsHandshake)
    }

    /// Whether this failure is a proxy tunnel/CONNECT error, which spider
    /// pairs with an independent local-DNS lookup before upgrading to a
    /// permanent NXDOMAIN (525) status.
    pub fn is_proxy_tunnel(&self) -> bool {
        matches!(self, EngineError::ProxyTunnel)
    }
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Other(msg) => write!(f, "engine error: {msg}"),
            other => write!(f, "engine error: {other:?}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// User-supplied in-process HTTP fetch transport. When installed on a
/// [`Website`](crate::website::Website) via
/// [`with_fetch_engine`](crate::website::Website::with_fetch_engine),
/// spider invokes [`HttpFetchEngine::fetch`] in place of its inner
/// `client.get(url).send()` on the HTTP crawl paths, for every URL where
/// [`HttpFetchEngine::should_fetch`] returns `true`.
///
/// The surrounding retry / cache / watchdog / hedge machinery still runs,
/// so the engine only has to return the body + response metadata for one
/// request.
///
/// Cancellation: the future is dropped on spider's first-byte-timeout, so
/// honor drop semantics.
#[async_trait::async_trait]
pub trait HttpFetchEngine: Send + Sync + 'static {
    /// Fetch a single URL and return its response, or a categorized
    /// [`EngineError`] that spider maps onto its retry semantics.
    async fn fetch(&self, req: EngineRequest<'_>) -> Result<EngineResponse, EngineError>;

    /// Consulted per URL before the engine is used. Returning `false`
    /// makes spider fall through to its built-in reqwest fetch for that
    /// URL. Default: always use the engine. Rollout ramps / per-domain
    /// breakers / kill switches belong here, keeping spider neutral.
    fn should_fetch(&self, _url: &str) -> bool {
        true
    }
}

/// Type alias used internally by `Website` to store an installed engine.
/// `Arc<dyn ...>` keeps the slot tiny when unset (`None`).
pub type SharedHttpFetchEngine = Arc<dyn HttpFetchEngine>;

/// Borrow-only bundle threaded down to spider's leaf fetch primitives so
/// they can consult the engine and build an [`EngineRequest`]. Cheap to
/// construct (two references); never clones the configuration.
#[derive(Copy, Clone)]
pub struct EngineFetchCtx<'a> {
    /// The installed engine.
    pub engine: &'a SharedHttpFetchEngine,
    /// The crawl configuration to hand the engine for parity.
    pub configuration: &'a Configuration,
}

impl<'a> EngineFetchCtx<'a> {
    /// Construct a context from an engine + configuration reference.
    pub fn new(engine: &'a SharedHttpFetchEngine, configuration: &'a Configuration) -> Self {
        Self {
            engine,
            configuration,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every error category maps onto the synthetic status spider uses for
    /// the equivalent reqwest failure. Locks the mapping so a drift in
    /// either table is caught.
    #[test]
    fn engine_error_status_mapping() {
        assert_eq!(EngineError::Timeout.to_status_code().as_u16(), 524);
        assert_eq!(EngineError::TlsHandshake.to_status_code().as_u16(), 526);
        assert_eq!(EngineError::Dns.to_status_code().as_u16(), 525);
        assert_eq!(EngineError::ConnectRefused.to_status_code().as_u16(), 521);
        assert_eq!(EngineError::ConnectAborted.to_status_code().as_u16(), 522);
        assert_eq!(EngineError::ConnectReset.to_status_code().as_u16(), 523);
        assert_eq!(
            EngineError::AddressUnreachable.to_status_code().as_u16(),
            526
        );
        assert_eq!(EngineError::ProxyTunnel.to_status_code().as_u16(), 503);
        assert_eq!(EngineError::Body.to_status_code().as_u16(), 400);
        assert_eq!(EngineError::Request.to_status_code().as_u16(), 400);
        assert_eq!(EngineError::Status(200).to_status_code().as_u16(), 200);
        assert_eq!(EngineError::Status(429).to_status_code().as_u16(), 429);
        assert_eq!(
            EngineError::Other("boom".into()).to_status_code().as_u16(),
            599
        );
    }

    /// An out-of-range custom status falls back to the unknown-status code.
    #[test]
    fn engine_error_invalid_status_falls_back() {
        assert_eq!(EngineError::Status(999).to_status_code().as_u16(), 999);
        assert_eq!(EngineError::Status(0).to_status_code().as_u16(), 599);
    }

    #[test]
    fn handshake_and_tunnel_predicates() {
        assert!(EngineError::TlsHandshake.is_handshake_failure());
        assert!(!EngineError::Timeout.is_handshake_failure());
        assert!(EngineError::ProxyTunnel.is_proxy_tunnel());
        assert!(!EngineError::Dns.is_proxy_tunnel());
    }

    #[test]
    fn engine_method_default_is_get() {
        assert_eq!(EngineMethod::default(), EngineMethod::Get);
    }

    /// A minimal engine that returns a fixed body, exercising the trait
    /// object surface and the `should_fetch` default + override.
    struct MockEngine {
        accept: bool,
    }

    #[async_trait::async_trait]
    impl HttpFetchEngine for MockEngine {
        async fn fetch(&self, req: EngineRequest<'_>) -> Result<EngineResponse, EngineError> {
            Ok(EngineResponse {
                status_code: StatusCode::OK,
                final_url: Some(req.url.to_string()),
                headers: reqwest::header::HeaderMap::new(),
                body: b"<html>ok</html>".to_vec(),
                served: true,
                ..Default::default()
            })
        }

        fn should_fetch(&self, _url: &str) -> bool {
            self.accept
        }
    }

    #[test]
    fn mock_engine_is_object_safe_and_shareable() {
        let engine: SharedHttpFetchEngine = std::sync::Arc::new(MockEngine { accept: true });
        assert!(engine.should_fetch("https://example.com"));
        let declined: SharedHttpFetchEngine = std::sync::Arc::new(MockEngine { accept: false });
        assert!(!declined.should_fetch("https://example.com"));
    }

    #[tokio::test]
    async fn mock_engine_fetch_returns_body() {
        let engine = MockEngine { accept: true };
        let cfg = Configuration::default();
        let resp = engine
            .fetch(EngineRequest {
                url: "https://example.com",
                method: EngineMethod::Get,
                configuration: &cfg,
                only_html: true,
                attempt: 0,
                conditional_headers: None,
            })
            .await
            .expect("engine returns ok");
        assert_eq!(resp.status_code, StatusCode::OK);
        assert_eq!(resp.body, b"<html>ok</html>");
        assert_eq!(resp.final_url.as_deref(), Some("https://example.com"));
        assert!(resp.served);
    }
}
