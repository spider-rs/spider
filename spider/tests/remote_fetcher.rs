//! Integration tests for the `RemoteFetcher` hook on `Website::crawl`.
//!
//! Default behavior (no fetcher set) is covered by the rest of the test
//! suite — these tests confirm:
//!
//! 1. When a fetcher is installed, every per-URL fetch is delegated.
//! 2. Spider continues to drive link extraction + the subscription
//!    channel (`Page` events still flow).
//! 3. Spider's allow/deny/depth/visited tracking is still consulted —
//!    the fetcher only replaces the network round-trip.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use spider::fetcher::{FetchContext, RemoteFetcher};
use spider::utils::PageResponse;
use spider::website::Website;

/// Counting fetcher: records every URL it's asked for, then returns a
/// canned HTML payload referencing two outlinks under the same host.
#[derive(Default)]
struct CountingFetcher {
    calls: Arc<AtomicUsize>,
    seen: tokio::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl RemoteFetcher for CountingFetcher {
    async fn fetch(&self, ctx: FetchContext<'_>) -> PageResponse {
        self.calls.fetch_add(1, Ordering::SeqCst);
        {
            let mut seen = self.seen.lock().await;
            seen.push(ctx.url.to_string());
        }
        // Body advertises two outlinks. Spider's link extractor will
        // resolve them against the seed's host; we use absolute URLs
        // here to keep the test independent of base-href quirks.
        PageResponse {
            content: Some(
                "<html><body>\
                    <a href=\"https://example.test/a\">a</a>\
                    <a href=\"https://example.test/b\">b</a>\
                </body></html>"
                    .as_bytes()
                    .to_vec(),
            ),
            status_code: reqwest::StatusCode::OK,
            final_url: Some(ctx.url.to_string()),
            ..Default::default()
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_fetcher_is_invoked_and_drives_crawl() {
    let fetcher = Arc::new(CountingFetcher::default());
    let calls = fetcher.calls.clone();

    let mut site = Website::new("https://example.test/");
    site.with_depth(1);
    let mut budget = spider::hashbrown::HashMap::new();
    budget.insert("*", 5u32);
    site.with_budget(Some(budget));
    site.with_shared_remote_fetcher(fetcher.clone());

    // Subscribe BEFORE starting the crawl so we don't miss events.
    let mut rx = site.subscribe(16);
    let crawl_handle = tokio::spawn(async move {
        site.crawl().await;
    });

    let mut pages = Vec::new();
    while let Ok(page) = rx.recv().await {
        pages.push(page);
    }
    crawl_handle.await.unwrap();

    // Seed + two outlinks → fetcher should have been called at least 3
    // times. Spider's wildcard-budget off-by-one means budget=5 allows 4
    // through, so we get ≥3, ≤4.
    let n = calls.load(Ordering::SeqCst);
    assert!(
        (3..=4).contains(&n),
        "fetcher should be invoked 3-4 times (seed + outlinks), got {n}"
    );
    assert!(
        !pages.is_empty(),
        "subscription channel should have received Page events"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_fetcher_respects_wildcard_budget() {
    let fetcher = Arc::new(CountingFetcher::default());
    let calls = fetcher.calls.clone();

    let mut site = Website::new("https://example.test/");
    // Spider's wildcard budget: `*` = N allows N-1 URLs through
    // (off-by-one in `is_over_budget`). budget=2 → 1 URL.
    let mut budget = spider::hashbrown::HashMap::new();
    budget.insert("*", 2u32);
    site.with_budget(Some(budget));
    site.with_shared_remote_fetcher(fetcher.clone());

    let mut rx = site.subscribe(8);
    let crawl_handle = tokio::spawn(async move { site.crawl().await });
    while rx.recv().await.is_ok() {}
    crawl_handle.await.unwrap();

    let n = calls.load(Ordering::SeqCst);
    assert!(
        n <= 2,
        "wildcard budget=2 should cap fetches at ≤2 via spider's is_allowed gate, got {n}"
    );
    assert!(n >= 1, "should at least fetch the seed");
}
