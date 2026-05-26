//! Custom-fetch hook for [`crate::website::Website`].
//!
//! Implementing [`RemoteFetcher`] and installing it on a
//! [`Website`](crate::website::Website) via
//! [`with_remote_fetcher`](crate::website::Website::with_remote_fetcher)
//! reroutes spider's per-URL network round-trip through the user's code
//! while leaving every other crawl concern — visited tracking, depth,
//! allow/deny, robots, link extraction, scheduling, subscription
//! channels — in spider's hands.
//!
//! ## Default behavior unchanged
//!
//! A `Website` with **no** fetcher installed (the default, and the only
//! possibility on existing API users' code) runs the exact same fetch
//! path it always has — the built-in reqwest client, all feature-gated
//! retries / cache / hedge / parallel-backends machinery, everything.
//!
//! The hook is purely additive: it short-circuits *before* the built-in
//! fetch path executes, so when it fires none of those layers run. The
//! `RemoteFetcher` implementation owns those concerns on its own side
//! (gottem's orchestrator, for example, brings its own retry ladder /
//! escalation / hedge across cloud vendors).
//!
//! ## Scope (today)
//!
//! Today the hook fires only in the **HTTP** crawl path
//! ([`Website::crawl`](crate::website::Website::crawl) /
//! [`Website::crawl_raw`](crate::website::Website::crawl_raw)). The
//! chrome / webdriver / smart variants still drive their own
//! browser-backed fetches; setting a fetcher on a chrome-mode Website
//! has no effect there. Extending the hook to those paths is future
//! work — straightforward, but each path has its own machinery and
//! would expand the surface beyond what this addition is designed for.
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use spider::fetcher::{FetchContext, RemoteFetcher};
//! use spider::utils::PageResponse;
//! use spider::website::Website;
//!
//! struct MyFetcher;
//!
//! #[async_trait::async_trait]
//! impl RemoteFetcher for MyFetcher {
//!     async fn fetch(&self, ctx: FetchContext<'_>) -> PageResponse {
//!         // … call your transport (HTTP, gRPC, cloud API, anything) …
//!         // Return a PageResponse spider can consume.
//!         let mut resp = PageResponse::default();
//!         resp.final_url = Some(ctx.url.to_string());
//!         resp.content = Some(b"<html>hi</html>".to_vec());
//!         resp
//!     }
//! }
//!
//! # async fn ex() {
//! let mut site = Website::new("https://example.com");
//! site.with_remote_fetcher(MyFetcher);
//! site.crawl().await; // every per-URL fetch flows through MyFetcher
//! # }
//! ```

use std::sync::Arc;

use crate::configuration::Configuration;
use crate::utils::PageResponse;

/// Per-request context handed to a [`RemoteFetcher`]. Borrowed for the
/// duration of one fetch call.
#[derive(Debug)]
pub struct FetchContext<'a> {
    /// The target URL spider would have fetched.
    pub url: &'a str,
    /// The crawl-level configuration (allow/deny, user agent, timeout
    /// hints, etc.). The fetcher MAY consult fields like
    /// `user_agent` / `request_timeout` for parity with what spider would
    /// have used; nothing forces it to.
    pub configuration: &'a Configuration,
}

/// User-supplied fetch transport. When installed on a
/// [`Website`](crate::website::Website) via
/// [`with_remote_fetcher`](crate::website::Website::with_remote_fetcher),
/// spider invokes this on every URL that survives the `is_allowed`
/// gate, replacing its built-in reqwest fetch.
///
/// Cancellation: the fetcher implementation is responsible for honoring
/// any cancellation contract its caller provides. Spider does not pass
/// a cancel token into the trait — drop semantics on the future are the
/// signal.
#[async_trait::async_trait]
pub trait RemoteFetcher: Send + Sync + 'static {
    /// Fetch `ctx.url` and return a [`PageResponse`]. The returned
    /// response is fed straight into
    /// [`Page::build`](crate::page::build) — set
    /// [`PageResponse::content`] to the body bytes,
    /// [`PageResponse::final_url`] to the post-redirect URL, and
    /// [`PageResponse::status_code`] to the response status. Other
    /// fields (cookies, headers, etc.) are optional.
    async fn fetch(&self, ctx: FetchContext<'_>) -> PageResponse;
}

/// Type alias used internally by `Website` to store an installed
/// fetcher. `Arc<dyn ...>` keeps the slot tiny when unset (`None`).
pub type SharedRemoteFetcher = Arc<dyn RemoteFetcher>;
