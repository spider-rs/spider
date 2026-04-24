//! Smoke test for v2.51.117 smart-mode chrome-fetch path.
//!
//! Verifies that the `source_str` → `target_url` refactor in
//! fetch_page_html_chrome_base did not regress observable smart-mode
//! behavior: get_url() echoes the requested URL, final_redirect_destination
//! is either None or a real URL (never HTML garbage), and the subscribed
//! Page carries a real status code and non-empty body.
//!
//! Disabled by default via both `#[ignore]` and a `RUN_LIVE_TESTS`
//! env-var gate — reaches out to `react.dev` on the open internet and
//! launches a real Chrome, so it must be opt-in.
//!
//! Run:
//!   RUN_LIVE_TESTS=1 cargo test -p spider --test smart_smoke_v117 \
//!     --features "smart" --release -- --ignored --nocapture

#[cfg(all(feature = "chrome", feature = "smart"))]
mod smoke {
    use spider::tokio;
    use spider::website::Website;
    use std::time::Duration;

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

    /// Crawl via smart mode and collect all subscribed Page events.
    async fn smart_crawl_collect(url: &str, limit: u32) -> Vec<spider::page::Page> {
        let mut w = Website::new(url);
        w.with_limit(limit)
            .with_request_timeout(Some(Duration::from_secs(45)))
            .with_crawl_timeout(Some(Duration::from_secs(120)))
            .with_respect_robots_txt(false);

        let mut rx = w.subscribe(64);
        let collector = tokio::spawn(async move {
            let mut pages = Vec::new();
            while let Ok(p) = rx.recv().await {
                pages.push(p);
            }
            pages
        });

        w.crawl_smart().await;
        w.unsubscribe();

        collector.await.unwrap_or_default()
    }

    #[tokio::test]
    #[ignore = "live network test — opt in with --ignored and RUN_LIVE_TESTS=1"]
    async fn smart_crawl_react_dev_completes() {
        if !run_live_tests() {
            eprintln!("SKIP: set RUN_LIVE_TESTS=1");
            return;
        }
        let _ = env_logger::try_init();

        let url = "https://react.dev/";
        let pages = tokio::time::timeout(Duration::from_secs(150), smart_crawl_collect(url, 5))
            .await
            .expect("crawl should not timeout");

        eprintln!("=== smart_crawl_react_dev_completes ===");
        eprintln!("subscribed pages = {}", pages.len());

        assert!(
            !pages.is_empty(),
            "smart crawl should have emitted at least 1 subscribed page"
        );

        // The `source_str` → `target_url` refactor touched these fields for
        // pages that go through the chrome-upgrade path. Validate every
        // subscribed Page holds sensible values — never HTML-as-URL garbage.
        for (i, page) in pages.iter().enumerate() {
            let page_url = page.get_url().to_string();
            let final_url = page.final_redirect_destination.clone();
            let status = page.status_code.as_u16();
            let html_len = page.get_html_bytes_u8().len();

            eprintln!(
                "  [{}] url={} status={} final={:?} html_bytes={}",
                i, page_url, status, final_url, html_len
            );

            // get_url() must echo a valid URL, not HTML, and not be empty.
            assert!(
                page_url.starts_with("http://") || page_url.starts_with("https://"),
                "page.get_url() is not a URL: {:?}",
                page_url
            );
            assert!(
                !page_url.contains('<') && !page_url.contains('>'),
                "page.get_url() contains HTML markup (regression!): {:?}",
                page_url
            );

            // status must be a real HTTP code
            assert!(
                status >= 100 && status < 600,
                "invalid status code {} on {}",
                status,
                page_url
            );

            // final_redirect_destination: if Some, must be a URL and must
            // not contain HTML markup. Under the pre-v2.51.117 behavior in
            // smart mode, this field flowed through get_final_redirect
            // which received the decoded HTML body as its "source" URL
            // argument — we want to be sure the new target_url plumbing
            // never produces HTML-shaped values here.
            if let Some(fu) = final_url.as_deref() {
                let fu = fu.trim();
                if !fu.is_empty() {
                    assert!(
                        fu.starts_with("http://") || fu.starts_with("https://"),
                        "final_redirect_destination not a URL: {:?}",
                        fu
                    );
                    assert!(
                        !fu.contains('<') && !fu.contains('>'),
                        "final_redirect_destination contains HTML markup (regression!): {:?}",
                        fu
                    );
                }
            }

            // A 2xx/3xx page on a public SPA should have non-trivial HTML.
            if (200..400).contains(&status) {
                assert!(
                    html_len > 500,
                    "html too small for {} (status {}): {} bytes",
                    page_url,
                    status,
                    html_len
                );
            }
        }

        eprintln!("PASS — smart mode observable fields all valid");
    }
}
