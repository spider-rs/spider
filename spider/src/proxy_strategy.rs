//! Per-request proxy routing strategy.
//!
//! The [`ProxyStrategy`] trait is the proxy-side analogue of
//! [`crate::retry_strategy::RetryStrategy`]: an opt-in, per-request
//! decision hook that decides which categorical "kind" a request
//! belongs to. The crate then looks the kind up in
//! [`crate::configuration::Configuration::proxies_by_kind`] and routes
//! the request through the matching proxy list, lazily building a
//! secondary HTTP client on first use and dropping it automatically
//! once no in-flight request still holds a clone.
//!
//! This module is purely additive — no behavior changes when no
//! strategy is set on the [`Website`](crate::website::Website). Default
//! `proxy_strategy = None` keeps every existing call site identical to
//! today.
//!
//! # Composing with `RetryStrategy`
//!
//! [`RetryStrategy`](crate::retry_strategy::RetryStrategy) decides
//! *when* to retry and what overall configuration to apply.
//! `ProxyStrategy` decides *which proxy* a single request goes through.
//! The two compose: a retry strategy may swap the entire proxy list
//! between attempts; the proxy strategy then routes within whatever
//! list is current.
//!
//! # Concurrency
//!
//! [`ProxyStrategy::route`] takes `&self` and is shared across tasks
//! via [`SharedProxyStrategy`] (`Arc<dyn ProxyStrategy>`). Use atomics
//! or [`arc_swap`] inside your impl when you need per-strategy state —
//! mutexes are not required and not recommended on the hot path.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use spider::configuration::ProxyKind;
//! use spider::proxy_strategy::{ProxyStrategy, ProxyRouteContext, SharedProxyStrategy};
//! use spider::utils::media_asset::is_media_asset_url;
//!
//! struct AssetRouter;
//! impl ProxyStrategy for AssetRouter {
//!     fn route(&self, ctx: &ProxyRouteContext) -> ProxyKind {
//!         if is_media_asset_url(ctx.url) {
//!             ProxyKind::MediaAsset
//!         } else {
//!             ProxyKind::Default
//!         }
//!     }
//! }
//! let _strategy: SharedProxyStrategy = Arc::new(AssetRouter);
//! ```

use crate::configuration::ProxyKind;
use std::sync::Arc;

/// Borrowed context for a single routing decision.
///
/// All fields borrow from caller-owned storage so dispatching the
/// strategy is allocation-free. Add fields here in a backward-compatible
/// way (the struct is not `#[non_exhaustive]` yet — promote to that on
/// the next minor bump if desired).
#[derive(Debug)]
pub struct ProxyRouteContext<'a> {
    /// The URL the request will be sent to. Includes scheme, host, path,
    /// query, and fragment as written by the caller.
    pub url: &'a str,
    /// `0` for the initial attempt, `1+` for retries.
    pub attempt: u32,
    /// Profile key from the previous attempt, if any. Mirrors
    /// [`crate::retry_strategy::AttemptOutcome::profile_key`] so a
    /// single strategy can inspect retry context when present.
    pub previous_profile_key: Option<&'a str>,
}

/// A pluggable strategy for routing requests to a proxy kind.
///
/// Implementations are object-safe and shared via `Arc<dyn ProxyStrategy>`.
/// Returning [`ProxyKind::Default`] (or any kind not present in
/// [`crate::configuration::Configuration::proxies_by_kind`]) keeps the
/// existing fast path — no secondary client is built, no allocation,
/// no behavior change.
///
/// # Example
///
/// See the [module docs](self) for a worked example pairing this with
/// [`crate::utils::media_asset::is_media_asset_url`].
pub trait ProxyStrategy: Send + Sync + 'static {
    /// Decide which kind this request belongs to.
    ///
    /// Called at most once per request on the HTTP path (V1 — the
    /// chrome path is unchanged, see crate-level docs). Implementations
    /// must be cheap and side-effect-free; the trait is `&self` so
    /// state must live in atomics or other lock-free containers.
    fn route(&self, ctx: &ProxyRouteContext) -> ProxyKind;
}

/// Shared handle to a [`ProxyStrategy`], cheap to clone across tasks.
pub type SharedProxyStrategy = Arc<dyn ProxyStrategy>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact_str::CompactString;

    /// Static-routing strategy that always returns the same kind.
    struct Constant(ProxyKind);
    impl ProxyStrategy for Constant {
        fn route(&self, _: &ProxyRouteContext) -> ProxyKind {
            self.0.clone()
        }
    }

    /// Strategy that returns MediaAsset for URLs whose path ends in `.jpg`.
    /// Used purely as a tiny hand-rolled classifier — the real one lives
    /// in [`crate::utils::media_asset`].
    struct JpgRouter;
    impl ProxyStrategy for JpgRouter {
        fn route(&self, ctx: &ProxyRouteContext) -> ProxyKind {
            if ctx.url.ends_with(".jpg") {
                ProxyKind::MediaAsset
            } else {
                ProxyKind::Default
            }
        }
    }

    fn ctx<'a>(url: &'a str) -> ProxyRouteContext<'a> {
        ProxyRouteContext {
            url,
            attempt: 0,
            previous_profile_key: None,
        }
    }

    #[test]
    fn constant_returns_kind() {
        let s = Constant(ProxyKind::MediaAsset);
        assert_eq!(s.route(&ctx("https://example.com/page")), ProxyKind::MediaAsset);
        let s = Constant(ProxyKind::Default);
        assert_eq!(s.route(&ctx("https://example.com/page")), ProxyKind::Default);
        let s = Constant(ProxyKind::Custom(CompactString::new("tier-a")));
        match s.route(&ctx("https://example.com/page")) {
            ProxyKind::Custom(s) => assert_eq!(&*s, "tier-a"),
            other => panic!("unexpected kind {other:?}"),
        }
    }

    #[test]
    fn jpg_router_routes_media() {
        let s = JpgRouter;
        assert_eq!(
            s.route(&ctx("https://example.com/foo.jpg")),
            ProxyKind::MediaAsset
        );
        assert_eq!(
            s.route(&ctx("https://example.com/index.html")),
            ProxyKind::Default
        );
    }

    #[test]
    fn shared_strategy_dispatches_through_arc() {
        let s: SharedProxyStrategy = Arc::new(JpgRouter);
        let s2 = s.clone();
        let kind = s2.route(&ctx("https://example.com/x.jpg"));
        assert_eq!(kind, ProxyKind::MediaAsset);
    }
}
