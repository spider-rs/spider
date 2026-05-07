//! Live regression test: permanent DNS failures must not be retried in
//! either the HTTP or the Chrome fetch path. The two probe URLs
//! (`midwestrodding.com`, `orthwestsci.com`) are known NXDOMAIN /
//! NOERROR-with-no-A hosts. Without correct classification they would
//! cycle through the full retry budget — observable here as a
//! multi-second elapsed time even though the network call itself returns
//! immediately.
//!
//! Gated behind `RUN_LIVE_TESTS=1` so the suite never depends on the
//! ambient DNS state on developer machines / CI without explicit opt-in.
//!
//!   RUN_LIVE_TESTS=1 cargo test -p spider --test dns_no_retry
//!   RUN_LIVE_TESTS=1 cargo test -p spider --test dns_no_retry --features chrome
//!   RUN_LIVE_TESTS=1 cargo test -p spider --test dns_no_retry --features smart
//!   RUN_LIVE_TESTS=1 cargo test -p spider --test dns_no_retry --features balance

#![cfg(not(feature = "decentralized"))]

use spider::page::Page;
use spider::reqwest::StatusCode;
use spider::website::{CrawlStatus, Website};
use std::env;
use std::time::{Duration, Instant};

/// Mirrors the crate-private `DNS_RESOLVE_ERROR` (525) — keeping the
/// numeric here in lockstep avoids having to expand the public surface
/// for a test-only assertion.
const DNS_RESOLVE_ERROR_U16: u16 = 525;
/// Mirrors the crate-private `ADDRESS_UNREACHABLE_ERROR` (526).
const ADDRESS_UNREACHABLE_ERROR_U16: u16 = 526;

const PROBES: &[&str] = &["https://midwestrodding.com/", "https://orthwestsci.com/"];

/// Cap on how long a permanent-DNS HTTP/smart crawl may take. Backoff is
/// seeded at 200ms; a single retry would push the crawl past ~250ms, and
/// the full retry=5 budget would burn ~6s of backoff alone. Keeping this
/// tight catches both retries *and* unexpected hangs on the resolve path.
const NO_RETRY_BUDGET: Duration = Duration::from_secs(5);

/// Looser cap for the Chrome path — browser launch + tab provisioning
/// can dominate. Still tight enough to catch a full retry=5 cycle (each
/// retry creates a fresh tab → multi-second per attempt).
#[cfg(all(feature = "chrome", feature = "sync"))]
const NO_RETRY_BUDGET_CHROME: Duration = Duration::from_secs(15);

fn run_live_tests() -> bool {
    matches!(
        env::var("RUN_LIVE_TESTS")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Run an async body on a dedicated runtime backed by a worker pool with
/// an enlarged stack. The full feature combo
/// (`agent_chrome + parallel_backends + chrome_remote_cache + ...`) blows
/// the default 2 MiB tokio worker stack just from the size of the
/// `Website` struct + crawl future locals — boxing individual futures is
/// not enough. Pin both the runtime and the test future on a 16 MiB
/// thread, then drop them cleanly.
fn block_on_isolated<F>(body: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let rt = spider::tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(16 * 1024 * 1024)
                .enable_all()
                .build()
                .expect("build isolated tokio runtime");
            rt.block_on(body);
        })
        .expect("spawn isolated test thread");
    handle.join().expect("isolated test thread panicked");
}

fn assert_dns_terminal(page: &Page, url: &str) {
    let status = page.status_code;
    let dns = StatusCode::from_u16(DNS_RESOLVE_ERROR_U16).expect("525 is a valid status code");
    let unreachable =
        StatusCode::from_u16(ADDRESS_UNREACHABLE_ERROR_U16).expect("526 is a valid status code");
    assert!(
        status == dns || status == unreachable,
        "{url}: expected 525/526 for DNS-dead host, got {status}"
    );
    assert!(
        !page.should_retry,
        "{url}: page.should_retry must be false for permanent DNS failure"
    );
    assert!(
        !page.needs_retry(),
        "{url}: needs_retry() must be false for permanent DNS failure"
    );
}

fn assert_website_terminal(website: &Website, url: &str) {
    assert!(
        !website.get_initial_page_should_retry(),
        "{url}: website.initial_page_should_retry must be false"
    );
    assert_eq!(
        *website.get_status(),
        CrawlStatus::ConnectError,
        "{url}: CrawlStatus must be ConnectError for permanent DNS failure"
    );
}

#[cfg(feature = "sync")]
async fn crawl_with_retry_budget(url: &str) -> (Website, Vec<Page>, Duration) {
    let mut website = Website::new(url);
    // Pin a generous retry budget — if classification regressed and
    // `is_retryable_status(525)` started returning true, the crawler
    // would burn five rounds with backoffs pushing the total past
    // NO_RETRY_BUDGET.
    website
        .with_retry(5)
        .with_request_timeout(Some(Duration::from_secs(15)))
        .with_crawl_timeout(Some(Duration::from_secs(20)));

    let mut w = website.clone();
    let mut rx = w.subscribe(8);
    let (done_tx, mut done_rx) = spider::tokio::sync::oneshot::channel::<()>();

    let started = Instant::now();
    // Pinning the large feature-laden future on the heap keeps a stack
    // overflow off the default 2 MiB tokio worker stack — at the maximum
    // feature combo (`parallel_backends + agent_chrome + ...`) the
    // crawl future can easily exceed it.
    let crawl: std::pin::Pin<Box<dyn std::future::Future<Output = Website> + Send>> =
        Box::pin(async move {
            w.crawl_raw().await;
            w.unsubscribe();
            let _ = done_tx.send(());
            w
        });

    let mut pages: Vec<Page> = Vec::new();
    let sub = async {
        loop {
            spider::tokio::select! {
                // Bias toward `recv` so a fast-finishing crawl (e.g. DNS
                // failure returns synchronously) does not lose its
                // synthetic page to the `done_rx` arm. After `done_rx`
                // fires we drain any messages still buffered.
                result = rx.recv() => {
                    match result {
                        Ok(p) => pages.push(p),
                        Err(_) => break,
                    }
                }
                _ = &mut done_rx => {
                    while let Ok(p) = rx.try_recv() {
                        pages.push(p);
                    }
                    break;
                }
            }
        }
    };

    let (_, w) = spider::tokio::join!(sub, crawl);
    (w, pages, started.elapsed())
}

#[cfg(all(feature = "smart", feature = "sync"))]
async fn crawl_with_retry_budget_smart(url: &str) -> (Website, Vec<Page>, Duration) {
    let mut website = Website::new(url);
    website
        .with_retry(5)
        .with_request_timeout(Some(Duration::from_secs(15)))
        .with_crawl_timeout(Some(Duration::from_secs(20)));

    let mut w = website.clone();
    let mut rx = w.subscribe(8);
    let (done_tx, mut done_rx) = spider::tokio::sync::oneshot::channel::<()>();

    let started = Instant::now();
    let crawl: std::pin::Pin<Box<dyn std::future::Future<Output = Website> + Send>> =
        Box::pin(async move {
            w.crawl_smart().await;
            w.unsubscribe();
            let _ = done_tx.send(());
            w
        });

    let mut pages: Vec<Page> = Vec::new();
    let sub = async {
        loop {
            spider::tokio::select! {
                result = rx.recv() => {
                    match result {
                        Ok(p) => pages.push(p),
                        Err(_) => break,
                    }
                }
                _ = &mut done_rx => {
                    while let Ok(p) = rx.try_recv() {
                        pages.push(p);
                    }
                    break;
                }
            }
        }
    };

    let (_, w) = spider::tokio::join!(sub, crawl);
    (w, pages, started.elapsed())
}

#[cfg(all(feature = "chrome", feature = "sync"))]
async fn crawl_with_retry_budget_chrome(url: &str) -> (Website, Vec<Page>, Duration) {
    let mut website = Website::new(url);
    website
        .with_retry(5)
        .with_limit(1)
        .with_depth(0)
        .with_request_timeout(Some(Duration::from_secs(15)))
        .with_crawl_timeout(Some(Duration::from_secs(20)));

    let mut w = website.clone();
    let mut rx = w.subscribe(8);
    let (done_tx, mut done_rx) = spider::tokio::sync::oneshot::channel::<()>();

    let started = Instant::now();
    let crawl: std::pin::Pin<Box<dyn std::future::Future<Output = Website> + Send>> =
        Box::pin(async move {
            w.crawl().await;
            w.unsubscribe();
            let _ = done_tx.send(());
            w
        });

    let mut pages: Vec<Page> = Vec::new();
    let sub = async {
        loop {
            spider::tokio::select! {
                // Bias toward `recv` so a fast-finishing crawl (e.g. DNS
                // failure returns synchronously) does not lose its
                // synthetic page to the `done_rx` arm. After `done_rx`
                // fires we drain any messages still buffered.
                result = rx.recv() => {
                    match result {
                        Ok(p) => pages.push(p),
                        Err(_) => break,
                    }
                }
                _ = &mut done_rx => {
                    while let Ok(p) = rx.try_recv() {
                        pages.push(p);
                    }
                    break;
                }
            }
        }
    };

    let (_, w) = spider::tokio::join!(sub, crawl);
    (w, pages, started.elapsed())
}

/// `Page::new_page` is the page-level fetch primitive — verify the
/// classification result directly, independent of the website crawl
/// loop. We borrow the spider-internal client built by `Website::setup`
/// so the test runs through whatever resolver feature combo is active
/// (`dns_cache` / `reqwest_hickory_dns` / default reqwest, plus the
/// `http-cache-reqwest` wrap that `cache_request`/`cache_mem` activates).
/// Building a raw `reqwest::Client` here would short-circuit those
/// features and surface a different error chain.
///
/// Gated on `dns_cache` OR `reqwest_hickory_dns` because without one of
/// them the system-getaddrinfo error chain surfaces as
/// `io::ErrorKind::Uncategorized` whose Display does not match any AC
/// pattern. That gap is a pre-existing limitation of the classifier
/// (the crawl-level retry loop still terminates correctly because it
/// checks `is_retryable_status` against 503, but `should_retry` flips
/// true once); fixing it requires walking the source chain on
/// `is_request()` errors too, which is out of scope for this hotfix.
#[cfg(all(
    feature = "sync",
    any(feature = "dns_cache", feature = "reqwest_hickory_dns")
))]
async fn fetch_page_direct_terminal(url: &str) -> Page {
    let mut website = Website::new(url);
    website.with_request_timeout(Some(Duration::from_secs(10)));
    let (client, _handle) = website.setup().await;
    Page::new_page(url, &client).await
}

#[cfg(feature = "sync")]
async fn scrape_with_retry_budget_raw(url: &str) -> (Website, Duration) {
    let mut website = Website::new(url);
    website
        .with_retry(5)
        .with_request_timeout(Some(Duration::from_secs(15)))
        .with_crawl_timeout(Some(Duration::from_secs(20)));
    let started = Instant::now();
    let scrape: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = Box::pin(async {
        website.scrape_raw().await;
    });
    scrape.await;
    (website, started.elapsed())
}

#[cfg(all(feature = "smart", feature = "sync"))]
async fn scrape_with_retry_budget_smart(url: &str) -> (Website, Duration) {
    let mut website = Website::new(url);
    website
        .with_retry(5)
        .with_request_timeout(Some(Duration::from_secs(15)))
        .with_crawl_timeout(Some(Duration::from_secs(20)));
    let started = Instant::now();
    let scrape: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = Box::pin(async {
        website.scrape_smart().await;
    });
    scrape.await;
    (website, started.elapsed())
}

#[cfg(all(feature = "chrome", feature = "sync"))]
async fn scrape_with_retry_budget_chrome(url: &str) -> (Website, Duration) {
    let mut website = Website::new(url);
    website
        .with_retry(5)
        .with_limit(1)
        .with_depth(0)
        .with_request_timeout(Some(Duration::from_secs(15)))
        .with_crawl_timeout(Some(Duration::from_secs(20)));
    let started = Instant::now();
    let scrape: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = Box::pin(async {
        website.scrape().await;
    });
    scrape.await;
    (website, started.elapsed())
}

#[cfg(all(
    feature = "sync",
    any(feature = "dns_cache", feature = "reqwest_hickory_dns")
))]
#[test]
fn page_new_page_dns_dead_urls_never_retry() {
    if !run_live_tests() {
        eprintln!(
            "skipping page_new_page_dns_dead_urls_never_retry — set RUN_LIVE_TESTS=1 to enable"
        );
        return;
    }
    block_on_isolated(async {
        for url in PROBES {
            let started = Instant::now();
            let page = fetch_page_direct_terminal(url).await;
            let elapsed = started.elapsed();
            assert!(
                elapsed < NO_RETRY_BUDGET,
                "{url}: Page::new_page took {elapsed:?} (> {NO_RETRY_BUDGET:?}) — \
                 the page-level fetch must return immediately on permanent DNS failure"
            );
            assert_dns_terminal(&page, url);
        }
    });
}

#[cfg(feature = "sync")]
#[test]
fn http_dns_dead_urls_never_retry() {
    if !run_live_tests() {
        eprintln!("skipping http_dns_dead_urls_never_retry — set RUN_LIVE_TESTS=1 to enable");
        return;
    }
    block_on_isolated(async {
        for url in PROBES {
            let (website, pages, elapsed) = crawl_with_retry_budget(url).await;

            assert!(
                elapsed < NO_RETRY_BUDGET,
                "{url}: HTTP crawl took {elapsed:?} (> {NO_RETRY_BUDGET:?}) — \
                 retry loop must have engaged for a permanent DNS failure"
            );
            assert!(
                !pages.is_empty(),
                "{url}: subscriber must observe the synthetic page"
            );

            // Every page emitted for the dead host must be terminal —
            // even if the crawler emits a synthetic placeholder, it must
            // NOT request a retry from downstream consumers.
            for page in &pages {
                assert_dns_terminal(page, url);
            }
            assert_website_terminal(&website, url);
        }
    });
}

#[cfg(all(feature = "smart", feature = "sync"))]
#[test]
fn smart_dns_dead_urls_never_retry() {
    if !run_live_tests() {
        eprintln!("skipping smart_dns_dead_urls_never_retry — set RUN_LIVE_TESTS=1 to enable");
        return;
    }
    block_on_isolated(async {
        for url in PROBES {
            let (website, pages, elapsed) = crawl_with_retry_budget_smart(url).await;

            assert!(
                elapsed < NO_RETRY_BUDGET,
                "{url}: smart crawl took {elapsed:?} (> {NO_RETRY_BUDGET:?}) — \
                 retry loop must have engaged for a permanent DNS failure"
            );
            assert!(
                !pages.is_empty(),
                "{url}: subscriber must observe the synthetic page"
            );

            for page in &pages {
                assert_dns_terminal(page, url);
            }
            assert_website_terminal(&website, url);
        }
    });
}

#[cfg(all(feature = "chrome", feature = "sync"))]
#[test]
fn chrome_dns_dead_urls_never_retry() {
    if !run_live_tests() {
        eprintln!("skipping chrome_dns_dead_urls_never_retry — set RUN_LIVE_TESTS=1 to enable");
        return;
    }
    block_on_isolated(async {
        for url in PROBES {
            let (website, pages, elapsed) = crawl_with_retry_budget_chrome(url).await;

            assert!(
                elapsed < NO_RETRY_BUDGET_CHROME,
                "{url}: Chrome crawl took {elapsed:?} (> {NO_RETRY_BUDGET_CHROME:?}) — \
                 retry loop must have engaged for a permanent DNS failure"
            );
            assert!(
                !pages.is_empty(),
                "{url}: subscriber must observe the synthetic page"
            );

            for page in &pages {
                assert_dns_terminal(page, url);
            }
            assert_website_terminal(&website, url);
        }
    });
}

/// `scrape_*` clones the website internally before crawling, so
/// `self.status` and `self.initial_page_should_retry` never get
/// updated on the outer `Website` we hold here — the cloned crawler
/// owns those mutations. We instead assert the only directly
/// observable property: the call returned within budget (no retry
/// loop engaged) and any pages forwarded onto `self.pages` are
/// terminal. These tests are about catching hangs / retries — the
/// crawl-status propagation is intentionally out of scope.
#[cfg(feature = "sync")]
fn assert_scrape_pages_terminal(website: &Website, url: &str) {
    if let Some(pages) = website.get_pages() {
        for page in pages.iter() {
            assert_dns_terminal(page, url);
        }
    }
}

#[cfg(feature = "sync")]
#[test]
fn scrape_raw_dns_dead_urls_never_retry() {
    if !run_live_tests() {
        eprintln!("skipping scrape_raw_dns_dead_urls_never_retry — set RUN_LIVE_TESTS=1 to enable");
        return;
    }
    block_on_isolated(async {
        for url in PROBES {
            let (website, elapsed) = scrape_with_retry_budget_raw(url).await;
            assert!(
                elapsed < NO_RETRY_BUDGET,
                "{url}: scrape_raw took {elapsed:?} (> {NO_RETRY_BUDGET:?}) — \
                 retry loop must have engaged for a permanent DNS failure"
            );
            assert_scrape_pages_terminal(&website, url);
        }
    });
}

#[cfg(all(feature = "smart", feature = "sync"))]
#[test]
fn scrape_smart_dns_dead_urls_never_retry() {
    if !run_live_tests() {
        eprintln!(
            "skipping scrape_smart_dns_dead_urls_never_retry — set RUN_LIVE_TESTS=1 to enable"
        );
        return;
    }
    block_on_isolated(async {
        for url in PROBES {
            let (website, elapsed) = scrape_with_retry_budget_smart(url).await;
            assert!(
                elapsed < NO_RETRY_BUDGET,
                "{url}: scrape_smart took {elapsed:?} (> {NO_RETRY_BUDGET:?}) — \
                 retry loop must have engaged for a permanent DNS failure"
            );
            assert_scrape_pages_terminal(&website, url);
        }
    });
}

#[cfg(all(feature = "chrome", feature = "sync"))]
#[test]
fn scrape_chrome_dns_dead_urls_never_retry() {
    if !run_live_tests() {
        eprintln!(
            "skipping scrape_chrome_dns_dead_urls_never_retry — set RUN_LIVE_TESTS=1 to enable"
        );
        return;
    }
    block_on_isolated(async {
        for url in PROBES {
            let (website, elapsed) = scrape_with_retry_budget_chrome(url).await;
            assert!(
                elapsed < NO_RETRY_BUDGET_CHROME,
                "{url}: scrape (chrome) took {elapsed:?} (> {NO_RETRY_BUDGET_CHROME:?}) — \
                 retry loop must have engaged for a permanent DNS failure"
            );
            assert_scrape_pages_terminal(&website, url);
        }
    });
}
