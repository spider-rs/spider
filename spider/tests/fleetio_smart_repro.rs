//! End-to-end repro for cross-domain redirect handling in smart-mode crawl.
//!
//! `fleet.io` 301-redirects to `www.fleetio.com`. The rendered Next.js
//! homepage uses host-relative `<a href="/foo">` links throughout. Before
//! the fix, `crawl_establish_smart` extracted those links with a base of
//! `fleet.io` (the original input) instead of `www.fleetio.com` (the
//! post-redirect host) — every relative link 404'd against `fleet.io`.
//!
//! Run: RUN_LIVE_TESTS=1 cargo test -p spider --test fleetio_smart_repro \
//!      --features "smart chrome chrome_intercept" --release -- --ignored --nocapture

#[cfg(all(feature = "chrome", feature = "smart"))]
mod repro {
    use spider::tokio;
    use spider::website::Website;
    use std::time::{Duration, Instant};

    fn run_live_tests() -> bool {
        matches!(
            std::env::var("RUN_LIVE_TESTS")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        )
    }

    async fn collect_smart_pages(
        url: &str,
        limit: u32,
        external_domains: Option<Vec<String>>,
    ) -> (Vec<spider::page::Page>, bool, Duration) {
        let mut w = Website::new(url);
        w.with_limit(limit)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .with_crawl_timeout(Some(Duration::from_secs(60)))
            .with_respect_robots_txt(false);
        if let Some(ext) = external_domains {
            w.with_external_domains(Some(ext.into_iter()));
        }

        let mut rx = w.subscribe(64);
        let collector = tokio::spawn(async move {
            let mut pages = Vec::new();
            while let Ok(p) = rx.recv().await {
                eprintln!(
                    "  page url={} status={} bytes={} final_redirect={:?}",
                    p.get_url(),
                    p.status_code.as_u16(),
                    p.get_html_bytes_u8().len(),
                    p.final_redirect_destination
                );
                pages.push(p);
            }
            pages
        });

        let start = Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(120), w.crawl_smart()).await;
        let elapsed = start.elapsed();
        w.unsubscribe();
        let pages = collector.await.unwrap_or_default();

        (pages, outcome.is_ok(), elapsed)
    }

    /// End-to-end: cross-domain redirect rewrites the resolution base for
    /// extracted relative links, the crawl finishes promptly, and no
    /// stale pre-redirect-host URLs leak into the visited set.
    #[tokio::test]
    #[ignore]
    async fn smart_fleet_io_cross_domain_redirect_end_to_end() {
        if !run_live_tests() {
            eprintln!("SKIP: set RUN_LIVE_TESTS=1");
            return;
        }
        let _ = env_logger::try_init();

        eprintln!("=== smart_fleet_io_cross_domain_redirect_end_to_end ===");

        // Variant A: user pre-declares the redirected host as external.
        eprintln!("--- A: with declared external domain ---");
        let (pages_a, ok_a, elapsed_a) = collect_smart_pages(
            "https://fleet.io/",
            4,
            Some(vec!["https://www.fleetio.com/".to_string()]),
        )
        .await;
        eprintln!(
            "elapsed={:?} pages={} ok={}",
            elapsed_a,
            pages_a.len(),
            ok_a
        );

        // Variant B: user does NOT declare the redirected host. The
        // redirect-aware selector update has to enrol it automatically
        // by swapping `domain_parsed`/`base.0`/`base.1` over to the
        // post-redirect host.
        eprintln!("--- B: without declared external domain ---");
        let (pages_b, ok_b, elapsed_b) = collect_smart_pages("https://fleet.io/", 4, None).await;
        eprintln!(
            "elapsed={:?} pages={} ok={}",
            elapsed_b,
            pages_b.len(),
            ok_b
        );

        assert!(ok_a, "crawl_smart hung past 120s (A)");
        assert!(ok_b, "crawl_smart hung past 120s (B)");
        assert!(
            elapsed_a < Duration::from_secs(90) && elapsed_b < Duration::from_secs(90),
            "smart crawl too slow (A={:?}, B={:?})",
            elapsed_a,
            elapsed_b
        );

        // 1. Seed page must show the redirect destination + non-trivial body.
        let seed = pages_a
            .iter()
            .find(|p| p.get_url() == "https://fleet.io/")
            .expect("seed page must be in subscriber stream");
        assert_eq!(
            seed.status_code.as_u16(),
            200,
            "seed page status not 200: {}",
            seed.status_code
        );
        assert_eq!(
            seed.final_redirect_destination.as_deref(),
            Some("https://www.fleetio.com/"),
            "seed final_redirect must be www.fleetio.com"
        );
        assert!(
            seed.get_html_bytes_u8().len() > 50_000,
            "seed body too small: {} bytes",
            seed.get_html_bytes_u8().len()
        );

        // 2. Every page beyond the seed must live on www.fleetio.com — pre-fix,
        //    site-relative `/foo` links from the rendered fleetio.com homepage
        //    were resolved against `fleet.io` and 404'd.
        let mut secondary_a = 0usize;
        for page in pages_a
            .iter()
            .filter(|p| p.get_url() != "https://fleet.io/")
        {
            secondary_a += 1;
            let url = page.get_url().to_string();
            assert!(
                url.starts_with("https://www.fleetio.com/")
                    || url.starts_with("https://fleetio.com/"),
                "secondary page on wrong host (regression!): {}",
                url
            );
        }
        assert!(
            secondary_a >= 1,
            "expected ≥1 secondary page after redirect (A); got {}",
            pages_a.len()
        );

        // 3. Same expectation when the user did NOT declare the redirect host —
        //    `modify_selectors` running before extraction must enrol it.
        let mut secondary_b = 0usize;
        for page in pages_b
            .iter()
            .filter(|p| p.get_url() != "https://fleet.io/")
        {
            secondary_b += 1;
            let url = page.get_url().to_string();
            assert!(
                url.starts_with("https://www.fleetio.com/")
                    || url.starts_with("https://fleetio.com/"),
                "secondary page leaked off the redirect host (B): {}",
                url
            );
        }
        assert!(
            secondary_b >= 1,
            "expected ≥1 secondary page after redirect (B); got {}",
            pages_b.len()
        );
    }

    /// Sanity check that the smart-mode link extractor for redirected
    /// pages produces the expected post-redirect URLs even with a single
    /// `with_limit(1)` (single-page-mode skips link extraction unless
    /// `return_page_links` is set, which we set explicitly here).
    #[tokio::test]
    #[ignore]
    async fn smart_fleet_io_single_page_return_page_links() {
        if !run_live_tests() {
            eprintln!("SKIP: set RUN_LIVE_TESTS=1");
            return;
        }
        let _ = env_logger::try_init();

        let mut w = Website::new("https://fleet.io/");
        w.with_limit(1)
            .with_return_page_links(true)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .with_crawl_timeout(Some(Duration::from_secs(45)))
            .with_respect_robots_txt(false);

        let mut rx = w.subscribe(8);
        let collector = tokio::spawn(async move {
            let mut pages = Vec::new();
            while let Ok(p) = rx.recv().await {
                pages.push(p);
            }
            pages
        });

        let start = Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(60), w.crawl_smart()).await;
        let elapsed = start.elapsed();
        w.unsubscribe();
        let pages = collector.await.unwrap_or_default();

        eprintln!(
            "[single-page] elapsed={:?} pages={} ok={}",
            elapsed,
            pages.len(),
            outcome.is_ok()
        );
        assert!(outcome.is_ok(), "single-page crawl_smart hung past 60s");

        let seed = pages
            .iter()
            .find(|p| p.get_url() == "https://fleet.io/")
            .expect("seed page");
        let page_links = seed
            .page_links
            .as_ref()
            .expect("page_links should be populated when return_page_links=true");

        assert!(
            !page_links.is_empty(),
            "page_links is empty after redirect — extractor saw no hrefs"
        );

        // At least one extracted link must point at the redirect host.
        let any_redirect_host_link = page_links.iter().any(|cis| {
            let s: &str = cis.inner();
            s.starts_with("https://www.fleetio.com/") || s.starts_with("https://fleetio.com/")
        });
        let any_pre_redirect_link = page_links.iter().any(|cis| {
            let s: &str = cis.inner();
            (s.starts_with("https://fleet.io/") || s.starts_with("http://fleet.io/"))
                && s != "https://fleet.io/"
                && s != "http://fleet.io/"
        });

        eprintln!(
            "[single-page] extracted {} links, redirect_host_present={}, pre_redirect_present={}",
            page_links.len(),
            any_redirect_host_link,
            any_pre_redirect_link
        );

        assert!(
            any_redirect_host_link,
            "no extracted link resolved against the redirect host"
        );
        assert!(
            !any_pre_redirect_link,
            "site-relative links still resolving against pre-redirect host (regression!)"
        );
    }
}
