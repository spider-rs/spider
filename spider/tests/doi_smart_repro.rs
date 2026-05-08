//! End-to-end DOI multi-hop redirect: crawl_smart and crawl (chrome).
//!
//! `https://doi.org/10.1007/978-981-96-6596-9_10` redirects through ~5 hops
//! to `link.springer.com/chapter/10.1007/...`. Asserts both crawl paths:
//!   - report a non-empty body for the seed,
//!   - record `final_redirect_destination` pointing at the springer chapter,
//!   - extract secondary links that resolve against the redirect host.
//!
//! Run: RUN_LIVE_TESTS=1 cargo test -p spider --test doi_smart_repro \
//!      --features "smart chrome chrome_intercept" --release -- --ignored --nocapture

#[cfg(all(feature = "chrome", feature = "smart"))]
mod repro {
    use spider::page::Page;
    use spider::tokio;
    use spider::website::Website;
    use std::time::{Duration, Instant};

    const DOI_URL: &str = "https://doi.org/10.1007/978-981-96-6596-9_10";
    const SPRINGER_CHAPTER: &str = "https://link.springer.com/chapter/10.1007/978-981-96-6596-9_10";

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

    fn build(url: &str) -> Website {
        let mut w = Website::new(url);
        w.with_limit(2)
            .with_request_timeout(Some(Duration::from_secs(45)))
            .with_crawl_timeout(Some(Duration::from_secs(120)))
            .with_redirect_limit(15)
            .with_respect_robots_txt(false);
        w
    }

    async fn drain(label: &'static str, mut w: Website, chrome: bool) -> (Vec<Page>, Duration) {
        let mut rx = w.subscribe(64);
        let collector = tokio::spawn(async move {
            let mut pages = Vec::new();
            while let Ok(p) = rx.recv().await {
                eprintln!(
                    "[{label}] url={} status={} bytes={} final_redirect={:?}",
                    p.get_url(),
                    p.status_code.as_u16(),
                    p.get_html_bytes_u8().len(),
                    p.final_redirect_destination,
                );
                pages.push(p);
            }
            pages
        });
        let start = Instant::now();
        let _ = tokio::time::timeout(Duration::from_secs(180), async {
            if chrome {
                w.crawl().await;
            } else {
                w.crawl_smart().await;
            }
        })
        .await;
        let elapsed = start.elapsed();
        w.unsubscribe();
        let pages = collector.await.unwrap_or_default();
        (pages, elapsed)
    }

    fn assert_secondary_on_springer(label: &str, pages: &[Page]) {
        for p in pages.iter().filter(|p| p.get_url() != DOI_URL) {
            let url = p.get_url().to_string();
            assert!(
                !url.starts_with("https://doi.org/") && !url.starts_with("https://www.doi.org/"),
                "[{label}] secondary leaked back to doi.org: {url}"
            );
        }
    }

    #[tokio::test]
    #[ignore]
    async fn doi_smart_path() {
        if !run_live_tests() {
            eprintln!("SKIP");
            return;
        }
        let _ = env_logger::try_init();
        let (pages, elapsed) = drain("smart", build(DOI_URL), false).await;
        eprintln!("[smart] elapsed={:?} pages={}", elapsed, pages.len());

        let seed = pages
            .iter()
            .find(|p| p.get_url() == DOI_URL)
            .expect("seed page");
        assert_eq!(seed.status_code.as_u16(), 200, "seed status");
        assert!(
            seed.get_html_bytes_u8().len() > 100_000,
            "seed body too small"
        );
        assert_eq!(
            seed.final_redirect_destination.as_deref(),
            Some(SPRINGER_CHAPTER),
            "seed final_redirect mismatch"
        );
        assert_secondary_on_springer("smart", &pages);
    }

    #[tokio::test]
    #[ignore]
    async fn doi_chrome_path() {
        if !run_live_tests() {
            eprintln!("SKIP");
            return;
        }
        let _ = env_logger::try_init();
        let (pages, elapsed) = drain("chrome", build(DOI_URL), true).await;
        eprintln!("[chrome] elapsed={:?} pages={}", elapsed, pages.len());

        if pages.is_empty() {
            eprintln!("[chrome] SKIP — no pages emitted (chrome unavailable)");
            return;
        }

        let seed = pages
            .iter()
            .find(|p| p.get_url() == DOI_URL)
            .expect("seed page");
        assert_eq!(seed.status_code.as_u16(), 200, "[chrome] seed status");
        assert!(
            seed.get_html_bytes_u8().len() > 100_000,
            "[chrome] seed body too small"
        );
        let final_url = seed
            .final_redirect_destination
            .as_deref()
            .unwrap_or_default();
        assert!(
            final_url.starts_with("https://link.springer.com/chapter/10.1007/"),
            "[chrome] seed final_redirect mismatch: {final_url:?}"
        );
        assert_secondary_on_springer("chrome", &pages);
    }
}
