//! Regression: `Website::crawl` (HTTP path) through an HTTP proxy on an
//! NXDOMAIN target must exit in well under a second AND must not consult
//! the configured `RetryStrategy` even once. The leak this guards
//! against (fixed across v2.51.168 → v2.51.175) was that the async
//! tunnel-failure confirmation correctly classified status to 525, but
//! `build()`'s `get_error_status_base` re-classified the raw error and
//! flipped `should_retry=true`, so `needs_retry()` returned true and
//! the retry loop ran for many seconds × per-attempt connect timeout.
//!
//! Test runs an in-process fake HTTP proxy on loopback that returns
//! `HTTP/1.1 502 Bad Gateway` to every CONNECT — the same surface a
//! real proxy emits on target NXDOMAIN. Combined with an RFC 2606
//! `.invalid` target (guaranteed unresolvable by local DNS), the
//! two-signal confirmation path must classify as 525 and the retry
//! loop must short-circuit.
//!
//! Intentionally NOT gated on `RUN_LIVE_TESTS` — every signal is
//! synthetic (loopback proxy + .invalid target), so the test runs
//! deterministically on any machine without external network.
//!
//! Runs under both default features and `--features balance` (the
//! disk-spool variant) to guard the post-classification-and-spool
//! path that handles small NXDOMAIN responses differently.
//!
//!   cargo test -p spider --test proxy_retry_strategy_fast_exit
//!   cargo test -p spider --test proxy_retry_strategy_fast_exit --features balance

#![cfg(not(feature = "decentralized"))]

use spider::retry_strategy::{AttemptOutcome, RetryDirective, RetryStrategy};
use spider::website::Website;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Mirrors crate-private `DNS_RESOLVE_ERROR` (525).
const DNS_RESOLVE_ERROR_U16: u16 = 525;

/// Hard wall-clock cap for proxy + NXDOMAIN. Without the v2.51.168→v2.51.175
/// fixes this would routinely take 30+ seconds (per-attempt connect
/// timeout × N retries × exponential backoff). 1 second leaves comfortable
/// headroom over the observed ~175ms in-test path.
const FAST_EXIT_BUDGET: Duration = Duration::from_secs(1);

/// `RetryStrategy` that ALWAYS asks for a retry, with a high
/// `max_retries`. If `on_retry` is consulted even once for an NXDOMAIN
/// host, `consultations` will be > 0 — the test will fail. This is the
/// structural guarantee: `needs_retry()` must short-circuit BEFORE the
/// retry loop reaches the strategy.
struct AggressiveStrategy {
    consultations: Arc<AtomicU32>,
}

impl RetryStrategy for AggressiveStrategy {
    fn max_retries(&self) -> u32 {
        10
    }
    fn on_retry(&self, _outcome: &AttemptOutcome) -> RetryDirective {
        self.consultations.fetch_add(1, Ordering::SeqCst);
        RetryDirective {
            should_retry: true,
            ..Default::default()
        }
    }
}

/// Fake HTTP proxy on a loopback port. Responds to every CONNECT (or
/// any other request) with `502 Bad Gateway` — the same surface most
/// real proxies emit when their upstream DNS resolution returns
/// NXDOMAIN.
fn spawn_fake_502_proxy() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
            let _ = s.flush();
        }
    });
    port
}

/// Core scenario: crawl an NXDOMAIN host through a proxy with an
/// aggressive retry strategy configured. Must exit fast and never
/// consult the strategy.
#[tokio::test(flavor = "current_thread")]
async fn crawl_nxdomain_through_proxy_with_retry_strategy_exits_fast() {
    let proxy_port = spawn_fake_502_proxy();
    let proxy_url = format!("http://127.0.0.1:{proxy_port}");

    let consultations = Arc::new(AtomicU32::new(0));
    let strategy: Arc<dyn RetryStrategy> = Arc::new(AggressiveStrategy {
        consultations: Arc::clone(&consultations),
    });

    // RFC 2606 `.invalid` TLD — guaranteed unresolvable. Combined with
    // the fake proxy returning 502 to CONNECT, this synthesizes the
    // same chain as a real proxy + real NXDOMAIN target.
    let target = "https://this-host-must-not-exist.invalid/";
    let mut website = Website::new(target);
    website
        .with_limit(2)
        .with_retry(7) // even with default retry counter set, must not be used
        .with_proxies(Some(vec![proxy_url]))
        .with_request_timeout(Some(Duration::from_secs(3)))
        .with_retry_strategy(strategy);

    let start = Instant::now();
    website.crawl().await;
    let elapsed = start.elapsed();

    // (1) Must complete in well under one second.
    assert!(
        elapsed < FAST_EXIT_BUDGET,
        "crawl({target}) through proxy must exit in <{FAST_EXIT_BUDGET:?}; \
         got {elapsed:?}. Pre-fix this took 30+ seconds (retry budget × \
         backoff × per-attempt connect timeout).",
    );

    // (2) RetryStrategy::on_retry MUST NEVER be consulted. The retry loop
    // is gated on `while page.needs_retry() && retry_count > 0`; for
    // 525 / NXDOMAIN-confirmed-via-local-DNS, `needs_retry()` returns
    // false and the loop never enters its body, so the strategy is
    // never consulted regardless of `max_retries` or `should_retry`.
    let n = consultations.load(Ordering::SeqCst);
    assert_eq!(
        n, 0,
        "RetryStrategy::on_retry must NEVER be consulted on NXDOMAIN — \
         needs_retry() guards every retry loop in website.rs before \
         strategy lookup. Got {n} consultation(s).",
    );

    // (3) Resulting pages must classify to 525 with no retry signals.
    if let Some(pages) = website.get_pages() {
        let mut saw_seed = false;
        for page in pages.iter() {
            if page.get_url().contains("invalid") {
                saw_seed = true;
                assert_eq!(
                    page.status_code.as_u16(),
                    DNS_RESOLVE_ERROR_U16,
                    "page {} must classify to 525 (NXDOMAIN); got {}",
                    page.get_url(),
                    page.status_code,
                );
                assert!(
                    !page.should_retry,
                    "page {} must have should_retry=false — fix for \
                     get_error_status_base leak in v2.51.172 / v2.51.175",
                    page.get_url(),
                );
                assert!(
                    !page.needs_retry(),
                    "page {} needs_retry() must be false",
                    page.get_url(),
                );
            }
        }
        assert!(
            saw_seed,
            "expected at least one page result for the seed URL"
        );
    }
}

/// Same scenario, but with the legacy `with_retry` counter only (no
/// custom strategy). Verifies the gate is structural (in
/// `needs_retry()`, not in `RetryStrategy.on_retry`).
#[tokio::test(flavor = "current_thread")]
async fn crawl_nxdomain_through_proxy_legacy_retry_counter_exits_fast() {
    let proxy_port = spawn_fake_502_proxy();
    let proxy_url = format!("http://127.0.0.1:{proxy_port}");

    let target = "https://another-unresolvable-host.invalid/";
    let mut website = Website::new(target);
    website
        .with_limit(2)
        .with_retry(7)
        .with_proxies(Some(vec![proxy_url]))
        .with_request_timeout(Some(Duration::from_secs(3)));

    let start = Instant::now();
    website.crawl().await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < FAST_EXIT_BUDGET,
        "legacy-retry crawl({target}) through proxy must exit in \
         <{FAST_EXIT_BUDGET:?}; got {elapsed:?}",
    );
}

/// Resolvable host through the same fake proxy must STILL retry — the
/// proxy signal alone doesn't justify a permanent classification when
/// local DNS says the host exists. This is the v2.51.165 false-positive
/// regression case the v2.51.167 revert + v2.51.168 two-signal
/// confirmation closed.
#[tokio::test(flavor = "current_thread")]
async fn crawl_resolvable_host_through_failing_proxy_keeps_retrying() {
    let proxy_port = spawn_fake_502_proxy();
    let proxy_url = format!("http://127.0.0.1:{proxy_port}");

    // Limit retries so the test still terminates in bounded time even
    // though every attempt fails through the fake proxy. We're
    // asserting on retry COUNT > 0, not on success.
    let consultations = Arc::new(AtomicU32::new(0));
    struct CountingThenStop {
        n: Arc<AtomicU32>,
    }
    impl RetryStrategy for CountingThenStop {
        fn max_retries(&self) -> u32 {
            2
        }
        fn on_retry(&self, _: &AttemptOutcome) -> RetryDirective {
            self.n.fetch_add(1, Ordering::SeqCst);
            RetryDirective {
                should_retry: true,
                ..Default::default()
            }
        }
    }
    let strategy: Arc<dyn RetryStrategy> = Arc::new(CountingThenStop {
        n: Arc::clone(&consultations),
    });

    // localhost resolves locally — confirm helper will NOT upgrade to
    // 525, status stays 503 → retryable → retry loop enters → strategy
    // consulted.
    let target = "http://localhost/";
    let mut website = Website::new(target);
    website
        .with_limit(2)
        .with_proxies(Some(vec![proxy_url]))
        .with_request_timeout(Some(Duration::from_secs(2)))
        .with_retry_strategy(strategy);

    let start = Instant::now();
    website.crawl().await;
    let elapsed = start.elapsed();

    // Even with retries, resolvable-host failures still bound under
    // a few seconds. Backoff for the legacy retry path starts at 200ms
    // and doubles; with max_retries=2 we expect ~600ms-1.5s of backoff
    // plus per-attempt request time.
    assert!(
        elapsed < Duration::from_secs(15),
        "transient hiccup retry path must still terminate in bounded \
         time; got {elapsed:?}",
    );

    // The strategy MUST have been consulted at least once — proves the
    // permanent-classification fix did NOT accidentally suppress
    // retries for legitimate transient cases.
    let n = consultations.load(Ordering::SeqCst);
    assert!(
        n >= 1,
        "RetryStrategy::on_retry must be consulted at least once for \
         transient (resolvable-host) proxy hiccups — got {n}. If 0, the \
         v2.51.172/v2.51.175 fixes accidentally killed retries for \
         non-permanent statuses (regression of v2.51.165 false-positive \
         fix in the OPPOSITE direction)."
    );
}
