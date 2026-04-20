//! Configurable retry strategy for advanced retry logic.
//!
//! The [`RetryStrategy`] trait allows callers to control retry behavior
//! per attempt: switching proxies, browser connections, user agents,
//! fingerprinting, and more between retries. This is independent of the
//! simple [`Configuration::retry`](crate::configuration::Configuration::retry)
//! counter, which provides basic retry-with-backoff.
//!
//! When a `RetryStrategy` is set on a [`Website`](crate::website::Website),
//! it takes precedence over the `retry` counter.

use crate::compact_str::CompactString;
use crate::configuration::RequestProxy;
#[cfg(feature = "chrome")]
use crate::configuration::Viewport;
use crate::page::AntiBotTech;
use reqwest::StatusCode;
use std::sync::Arc;
use std::time::Duration;

/// Summary of a completed fetch attempt, passed to the strategy so it can
/// decide what to do next. All fields are borrowed for zero allocation.
#[derive(Debug)]
pub struct AttemptOutcome<'a> {
    /// Which retry attempt just completed (0 = initial fetch, 1 = first retry, ...).
    pub attempt: u32,
    /// HTTP status code of the response.
    pub status_code: StatusCode,
    /// Whether the page's own logic flagged this for retry.
    pub should_retry: bool,
    /// Whether the response body was truncated (stream error, chunk timeout).
    pub content_truncated: bool,
    /// Whether a WAF was detected.
    pub waf_check: bool,
    /// Which anti-bot technology was detected, if any.
    pub anti_bot_tech: &'a AntiBotTech,
    /// Whether a proxy was configured for this request.
    pub proxy_configured: bool,
    /// The URL that was fetched.
    pub url: &'a str,
    /// The profile key from the previous attempt, if any.
    pub profile_key: Option<&'a str>,
    /// Length of the HTML content in bytes. 0 means empty response.
    pub html_length: usize,
    /// Total bytes transferred for the page (from network events).
    pub bytes_transferred: Option<f64>,
    /// Error status string from the request, if any (e.g. connection errors).
    pub error_status: Option<&'a str>,
    /// The final redirect destination URL, if redirects were followed.
    pub final_redirect_destination: Option<&'a str>,
}

/// Instructions returned by a [`RetryStrategy`] to configure the next retry
/// attempt. Every field is `Option` so that `None` means "leave unchanged."
/// The strategy can return completely different configurations on each attempt
/// for dynamic per-retry profile switching.
#[derive(Debug)]
pub struct RetryDirective {
    /// Whether to retry at all. Set to `false` to stop retrying immediately.
    pub should_retry: bool,
    /// Override the proxy list for the next attempt.
    pub proxies: Option<Vec<RequestProxy>>,
    /// Override the chrome connection URL for the next attempt.
    #[cfg(feature = "chrome")]
    pub chrome_connection_url: Option<String>,
    /// Override multiple chrome connection URLs for failover.
    #[cfg(feature = "chrome")]
    pub chrome_connections: Option<Vec<String>>,
    /// Override the user agent string.
    pub user_agent: Option<CompactString>,
    /// Override the viewport dimensions.
    #[cfg(feature = "chrome")]
    pub viewport: Option<Viewport>,
    /// Override stealth mode.
    pub stealth: Option<bool>,
    /// Override fingerprinting.
    pub fingerprint: Option<bool>,
    /// Override JavaScript blocking (chrome intercept).
    #[cfg(feature = "chrome")]
    pub block_javascript: Option<bool>,
    /// Override analytics blocking (chrome intercept).
    #[cfg(feature = "chrome")]
    pub block_analytics: Option<bool>,
    /// Override idle network wait timeout (chrome).
    #[cfg(feature = "chrome")]
    pub wait_for_idle_network: Option<Option<crate::configuration::WaitForIdleNetwork>>,
    /// Override almost-idle network wait timeout (chrome).
    #[cfg(feature = "chrome")]
    pub wait_for_almost_idle_network: Option<Option<crate::configuration::WaitForIdleNetwork>>,
    /// Override idle DOM wait (chrome).
    #[cfg(feature = "chrome")]
    pub wait_for_idle_dom: Option<Option<crate::configuration::WaitForSelector>>,
    /// Override the request timeout.
    pub request_timeout: Option<Option<Duration>>,
    /// Override the referrer.
    pub referrer: Option<Option<String>>,
    /// Set extra info metadata (e.g. machine type identifier).
    #[cfg(feature = "extra_information")]
    pub extra_info: Option<Option<String>>,
    /// A profile key string for tracking which profile was used.
    /// Stored on the resulting [`Page::profile_key`](crate::page::Page::profile_key).
    pub profile_key: Option<CompactString>,
    /// Custom backoff override. `None` = use default exponential backoff.
    pub backoff: Option<Duration>,
    /// Per-attempt hedge (work-stealing) configuration override. `None` =
    /// leave the current [`Configuration::hedge`](crate::configuration::Configuration::hedge)
    /// untouched. `Some(cfg)` replaces it for the next attempt, enabling
    /// dynamic per-attempt hedge delays and concurrency. Set
    /// `HedgeConfig { enabled: false, .. }` to disable hedging for this attempt.
    #[cfg(feature = "hedge")]
    pub hedge: Option<crate::utils::hedge::HedgeConfig>,
    /// Whether to reset HTTP-related state before applying this directive.
    /// When `true`, calls `reset_status()` and `clear_headers()` before
    /// applying configuration changes. Does NOT clear crawl progress
    /// (visited links, pages, signatures, extra_links).
    /// Defaults to `false` to avoid accidentally wiping crawl state.
    pub reset_http_state: bool,
    /// Whether to rebuild the HTTP client after applying configuration
    /// changes. When `false` (default), skips client rebuild. Set to `true`
    /// when proxies or user agent change and you need the new client
    /// configuration to take effect.
    pub rebuild_client: bool,
}

impl Default for RetryDirective {
    fn default() -> Self {
        Self {
            should_retry: true,
            proxies: None,
            #[cfg(feature = "chrome")]
            chrome_connection_url: None,
            #[cfg(feature = "chrome")]
            chrome_connections: None,
            user_agent: None,
            #[cfg(feature = "chrome")]
            viewport: None,
            stealth: None,
            fingerprint: None,
            #[cfg(feature = "chrome")]
            block_javascript: None,
            #[cfg(feature = "chrome")]
            block_analytics: None,
            #[cfg(feature = "chrome")]
            wait_for_idle_network: None,
            #[cfg(feature = "chrome")]
            wait_for_almost_idle_network: None,
            #[cfg(feature = "chrome")]
            wait_for_idle_dom: None,
            request_timeout: None,
            referrer: None,
            #[cfg(feature = "extra_information")]
            extra_info: None,
            profile_key: None,
            backoff: None,
            #[cfg(feature = "hedge")]
            hedge: None,
            reset_http_state: false,
            rebuild_client: false,
        }
    }
}

impl RetryDirective {
    /// Create a directive that stops retrying.
    pub fn stop() -> Self {
        Self {
            should_retry: false,
            ..Default::default()
        }
    }

    /// Create a directive that continues with default settings.
    pub fn continue_default() -> Self {
        Default::default()
    }
}

/// A pluggable retry strategy that controls retry behavior per attempt.
///
/// Implementations decide how many retries to perform and what configuration
/// to apply on each attempt. The trait is object-safe and `Send + Sync + 'static`
/// so it can be shared across async crawl tasks via `Arc<dyn RetryStrategy>`.
///
/// When set on a [`Website`](crate::website::Website), the strategy replaces the
/// simple [`Configuration::retry`](crate::configuration::Configuration::retry) counter.
///
/// # Thread Safety
///
/// The trait uses `&self` because the strategy is shared via `Arc` across
/// concurrent crawl tasks. Use interior mutability (atomics, `Mutex`, etc.)
/// for per-task state tracking.
pub trait RetryStrategy: Send + Sync + 'static {
    /// Maximum number of retries this strategy allows.
    /// Replaces `Configuration::retry` when a custom strategy is set.
    fn max_retries(&self) -> u32;

    /// Called before each retry attempt (not before the initial attempt).
    /// Receives the outcome of the previous attempt and returns a directive
    /// describing how to configure the next attempt.
    ///
    /// Return `RetryDirective::stop()` to halt retrying.
    fn on_retry(&self, outcome: &AttemptOutcome) -> RetryDirective;
}

/// Shared handle to a retry strategy, cheaply cloneable across tasks.
pub type SharedRetryStrategy = Arc<dyn RetryStrategy>;

/// Apply a [`RetryDirective`] to a [`Website`](crate::website::Website),
/// mutating only the fields specified (non-`None`) in the directive.
///
/// When `directive.reset_http_state` is true, resets HTTP status and
/// clears headers before applying changes. When `directive.rebuild_client`
/// is true, rebuilds the HTTP client and headers after applying changes.
pub fn apply_directive(website: &mut crate::website::Website, directive: &RetryDirective) {
    // Reset HTTP-related state only (NOT crawl progress like links_visited/pages/signatures).
    if directive.reset_http_state {
        website.reset_status();
        website.clear_headers();
    }

    if let Some(ref proxies) = directive.proxies {
        website.with_proxies_direct(Some(proxies.clone()));
    }
    if let Some(ref ua) = directive.user_agent {
        website.with_user_agent(Some(ua.as_str()));
    }
    if let Some(stealth) = directive.stealth {
        website.with_stealth(stealth);
    }
    if let Some(fp) = directive.fingerprint {
        website.with_fingerprint(fp);
    }
    if let Some(ref rt) = directive.request_timeout {
        website.with_request_timeout(*rt);
    }
    if let Some(ref referrer) = directive.referrer {
        website.with_referrer(referrer.clone());
    }
    #[cfg(feature = "extra_information")]
    {
        if let Some(ref info) = directive.extra_info {
            website.set_extra_info(info.clone());
        }
    }
    #[cfg(feature = "chrome")]
    {
        if let Some(ref url) = directive.chrome_connection_url {
            website.with_chrome_connection(Some(url.clone()));
        }
        if let Some(ref urls) = directive.chrome_connections {
            website.with_chrome_connections(urls.clone());
        }
        if let Some(ref vp) = directive.viewport {
            website.with_viewport(Some(*vp));
        }
        if let Some(block_js) = directive.block_javascript {
            website.configuration.chrome_intercept.block_javascript = block_js;
        }
        if let Some(block_analytics) = directive.block_analytics {
            website.configuration.chrome_intercept.block_analytics = block_analytics;
        }
        if let Some(ref idle) = directive.wait_for_idle_network {
            website.with_wait_for_idle_network0(idle.clone());
        }
        if let Some(ref almost_idle) = directive.wait_for_almost_idle_network {
            website.with_wait_for_almost_idle_network0(almost_idle.clone());
        }
        if let Some(ref idle_dom) = directive.wait_for_idle_dom {
            website.with_wait_for_idle_dom(idle_dom.clone());
        }
    }

    #[cfg(feature = "hedge")]
    {
        if let Some(ref hedge_cfg) = directive.hedge {
            website.configuration.hedge = Some(hedge_cfg.clone());
        }
    }

    // Rebuild HTTP client and headers after config changes.
    if directive.rebuild_client {
        website.configure_headers();
        let client = website.configure_http_client();
        website.set_http_client(client);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A mock strategy that tracks calls and returns different configs per attempt.
    struct MockStrategy {
        max: u32,
        call_count: AtomicU32,
        stop_at: Option<u32>,
    }

    impl MockStrategy {
        fn new(max: u32, stop_at: Option<u32>) -> Self {
            Self {
                max,
                call_count: AtomicU32::new(0),
                stop_at,
            }
        }

        fn calls(&self) -> u32 {
            self.call_count.load(Ordering::Relaxed)
        }
    }

    impl RetryStrategy for MockStrategy {
        fn max_retries(&self) -> u32 {
            self.max
        }

        fn on_retry(&self, outcome: &AttemptOutcome) -> RetryDirective {
            self.call_count.fetch_add(1, Ordering::Relaxed);

            if let Some(stop) = self.stop_at {
                if outcome.attempt >= stop {
                    return RetryDirective::stop();
                }
            }

            RetryDirective {
                profile_key: Some(format!("profile_{}", outcome.attempt).into()),
                backoff: Some(Duration::from_millis(0)),
                // Vary config per attempt to prove dynamic switching works.
                stealth: Some(outcome.attempt > 2),
                fingerprint: Some(outcome.attempt > 1),
                reset_http_state: false,
                rebuild_client: false,
                ..Default::default()
            }
        }
    }

    #[test]
    fn test_directive_default_has_correct_booleans() {
        let d = RetryDirective::default();
        assert!(d.should_retry);
        assert!(!d.reset_http_state);
        assert!(!d.rebuild_client);
        assert!(d.proxies.is_none());
        assert!(d.user_agent.is_none());
        assert!(d.backoff.is_none());
        assert!(d.profile_key.is_none());
        #[cfg(feature = "hedge")]
        assert!(d.hedge.is_none());
    }

    #[test]
    fn test_directive_stop() {
        let d = RetryDirective::stop();
        assert!(!d.should_retry);
    }

    #[test]
    fn test_strategy_max_retries() {
        let s = MockStrategy::new(5, None);
        assert_eq!(s.max_retries(), 5);
    }

    #[test]
    fn test_strategy_on_retry_called_with_correct_attempt() {
        let s = MockStrategy::new(3, None);
        let outcome = AttemptOutcome {
            attempt: 2,
            status_code: StatusCode::INTERNAL_SERVER_ERROR,
            should_retry: true,
            content_truncated: false,
            waf_check: false,
            anti_bot_tech: &crate::page::AntiBotTech::None,
            proxy_configured: false,
            url: "https://example.com",
            profile_key: None,
            html_length: 0,
            bytes_transferred: None,
            error_status: None,
            final_redirect_destination: None,
        };
        let directive = s.on_retry(&outcome);
        assert!(directive.should_retry);
        assert_eq!(directive.profile_key.as_deref(), Some("profile_2"));
        assert_eq!(s.calls(), 1);
    }

    #[test]
    fn test_strategy_stops_when_requested() {
        let s = MockStrategy::new(10, Some(3));
        for i in 1..=5 {
            let outcome = AttemptOutcome {
                attempt: i,
                status_code: StatusCode::BAD_GATEWAY,
                should_retry: true,
                content_truncated: false,
                waf_check: false,
                anti_bot_tech: &crate::page::AntiBotTech::None,
                proxy_configured: false,
                url: "https://example.com",
                profile_key: None,
                html_length: 0,
                bytes_transferred: None,
                error_status: None,
                final_redirect_destination: None,
            };
            let d = s.on_retry(&outcome);
            if i >= 3 {
                assert!(!d.should_retry, "should stop at attempt {i}");
            } else {
                assert!(d.should_retry, "should continue at attempt {i}");
            }
        }
    }

    #[test]
    fn test_strategy_dynamic_config_per_attempt() {
        let s = MockStrategy::new(5, None);
        // Attempt 1: stealth=false, fingerprint=false
        let d = s.on_retry(&AttemptOutcome {
            attempt: 1,
            status_code: StatusCode::FORBIDDEN,
            should_retry: true,
            content_truncated: false,
            waf_check: true,
            anti_bot_tech: &crate::page::AntiBotTech::Cloudflare,
            proxy_configured: true,
            url: "https://example.com",
            profile_key: None,
            html_length: 0,
            bytes_transferred: Some(1024.0),
            error_status: None,
            final_redirect_destination: None,
        });
        assert_eq!(d.stealth, Some(false));
        assert_eq!(d.fingerprint, Some(false));

        // Attempt 3: stealth=true, fingerprint=true
        let d = s.on_retry(&AttemptOutcome {
            attempt: 3,
            status_code: StatusCode::FORBIDDEN,
            should_retry: true,
            content_truncated: false,
            waf_check: true,
            anti_bot_tech: &crate::page::AntiBotTech::Cloudflare,
            proxy_configured: true,
            url: "https://example.com",
            profile_key: Some("profile_1"),
            html_length: 500,
            bytes_transferred: Some(2048.0),
            error_status: None,
            final_redirect_destination: Some("https://example.com/blocked"),
        });
        assert_eq!(d.stealth, Some(true));
        assert_eq!(d.fingerprint, Some(true));
        assert_eq!(d.profile_key.as_deref(), Some("profile_3"));
    }

    #[test]
    fn test_strategy_shared_across_threads() {
        let mock = Arc::new(MockStrategy::new(3, None));
        let s: SharedRetryStrategy = mock.clone();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let s = s.clone();
                std::thread::spawn(move || {
                    let outcome = AttemptOutcome {
                        attempt: 1,
                        status_code: StatusCode::BAD_GATEWAY,
                        should_retry: true,
                        content_truncated: false,
                        waf_check: false,
                        anti_bot_tech: &crate::page::AntiBotTech::None,
                        proxy_configured: false,
                        url: "https://example.com",
                        profile_key: None,
                        html_length: 0,
                        bytes_transferred: None,
                        error_status: None,
                        final_redirect_destination: None,
                    };
                    s.on_retry(&outcome)
                })
            })
            .collect();

        for h in handles {
            let d = h.join().unwrap();
            assert!(d.should_retry);
        }

        // All 4 threads called on_retry
        assert_eq!(mock.calls(), 4);
    }

    #[test]
    fn test_apply_directive_no_panic_on_empty() {
        let mut website = crate::website::Website::new("https://example.com");
        let directive = RetryDirective {
            reset_http_state: false,
            rebuild_client: false,
            ..Default::default()
        };
        apply_directive(&mut website, &directive);
        // No fields changed, no panic
    }

    #[test]
    fn test_apply_directive_sets_user_agent() {
        let mut website = crate::website::Website::new("https://example.com");
        let directive = RetryDirective {
            user_agent: Some("TestBot/1.0".into()),
            reset_http_state: false,
            rebuild_client: false,
            ..Default::default()
        };
        apply_directive(&mut website, &directive);
        assert_eq!(
            website
                .configuration
                .user_agent
                .as_ref()
                .map(|b| b.as_str()),
            Some("TestBot/1.0")
        );
    }

    #[test]
    fn test_apply_directive_sets_proxies() {
        let mut website = crate::website::Website::new("https://example.com");
        let directive = RetryDirective {
            proxies: Some(vec![RequestProxy {
                addr: "http://proxy1:8080".into(),
                ..Default::default()
            }]),
            reset_http_state: false,
            rebuild_client: false,
            ..Default::default()
        };
        apply_directive(&mut website, &directive);
        assert!(website.configuration.proxies.is_some());
    }

    #[test]
    fn test_no_strategy_uses_config_retry() {
        // When no strategy is set, the retry_strategy field is None
        let website = crate::website::Website::new("https://example.com");
        assert!(website.retry_strategy.is_none());
        // Configuration::retry default is 0
        assert_eq!(website.configuration.retry, 0);
    }

    #[test]
    fn test_outcome_carries_all_fields() {
        let outcome = AttemptOutcome {
            attempt: 3,
            status_code: StatusCode::TOO_MANY_REQUESTS,
            should_retry: true,
            content_truncated: true,
            waf_check: true,
            anti_bot_tech: &crate::page::AntiBotTech::Cloudflare,
            proxy_configured: true,
            url: "https://example.com/page",
            profile_key: Some("proxy_tier_1"),
            html_length: 0,
            bytes_transferred: Some(512.0),
            error_status: Some("connection reset"),
            final_redirect_destination: Some("https://example.com/challenge"),
        };
        assert_eq!(outcome.attempt, 3);
        assert_eq!(outcome.html_length, 0);
        assert_eq!(outcome.bytes_transferred, Some(512.0));
        assert_eq!(outcome.error_status, Some("connection reset"));
        assert_eq!(
            outcome.final_redirect_destination,
            Some("https://example.com/challenge")
        );
    }
}
