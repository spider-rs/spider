//! Comparison test: crawl (chrome) vs crawl_smart on the same real-world URL.
//!
//! Verifies that smart mode produces HTML content comparable to full chrome
//! rendering on JS-heavy pages.
//!
//! Run:
//!   RUN_LIVE_TESTS=1 cargo test --test smart_vs_chrome --features "smart" -- --nocapture

#[cfg(all(feature = "chrome", feature = "smart"))]
mod compare {
    use spider::tokio;
    use spider::website::Website;
    use std::time::Duration;

    const URL: &str = "https://fastbots.ai/blog";
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
    const CRAWL_TIMEOUT: Duration = Duration::from_secs(60);

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

    fn build_website(url: &str) -> Website {
        let mut w = Website::new(url);
        w.with_limit(1)
            .with_depth(0)
            .with_request_timeout(Some(REQUEST_TIMEOUT))
            .with_crawl_timeout(Some(CRAWL_TIMEOUT))
            .with_respect_robots_txt(false);
        w
    }

    /// Crawl a URL via Chrome and collect the first page.
    async fn fetch_chrome(url: &str) -> Option<spider::page::Page> {
        let website = build_website(url);
        let mut w = website.clone();
        let mut rx = w.subscribe(4).expect("subscribe");
        let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();

        let crawl = async move {
            w.crawl().await;
            w.unsubscribe();
            let _ = done_tx.send(());
        };

        let mut page = None;
        let sub = async {
            loop {
                tokio::select! {
                    biased;
                    _ = &mut done_rx => break,
                    result = rx.recv() => {
                        if let Ok(p) = result {
                            page = Some(p);
                        } else {
                            break;
                        }
                    }
                }
            }
        };

        tokio::join!(sub, crawl);
        page
    }

    /// Crawl a URL via smart mode and collect the first page.
    async fn fetch_smart(url: &str) -> Option<spider::page::Page> {
        let website = build_website(url);
        let mut w = website.clone();
        let mut rx = w.subscribe(4).expect("subscribe");
        let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();

        let crawl = async move {
            w.crawl_smart().await;
            w.unsubscribe();
            let _ = done_tx.send(());
        };

        let mut page = None;
        let sub = async {
            loop {
                tokio::select! {
                    biased;
                    _ = &mut done_rx => break,
                    result = rx.recv() => {
                        if let Ok(p) = result {
                            page = Some(p);
                        } else {
                            break;
                        }
                    }
                }
            }
        };

        tokio::join!(sub, crawl);
        page
    }

    /// Extract visible text tokens (lowercased, deduplicated, len > 3) after stripping tags.
    fn text_tokens(html: &str) -> std::collections::HashSet<String> {
        let mut out = String::with_capacity(html.len());
        let mut in_tag = false;
        for ch in html.chars() {
            match ch {
                '<' => in_tag = true,
                '>' => {
                    in_tag = false;
                    out.push(' ');
                }
                _ if !in_tag => out.push(ch),
                _ => {}
            }
        }
        out.split_whitespace()
            .filter(|w| w.len() > 3)
            .map(|w| w.to_lowercase())
            .collect()
    }

    #[tokio::test]
    async fn fastbots_blog_crawl_chrome() {
        if !run_live_tests() {
            eprintln!("SKIP: set RUN_LIVE_TESTS=1 to run");
            return;
        }

        let _ = env_logger::try_init();

        let result = tokio::time::timeout(Duration::from_secs(90), fetch_chrome(URL)).await;
        assert!(result.is_ok(), "crawl() should not timeout");

        if let Some(page) = result.unwrap() {
            let html = page.get_html();
            let status = page.status_code.as_u16();
            eprintln!("crawl() chrome: {} bytes, status={}", html.len(), status);
            assert!(
                html.len() > 1000,
                "crawl() HTML too small: {} bytes (status={})",
                html.len(),
                status
            );
        } else {
            eprintln!("SKIP: crawl() returned no page (chrome unavailable)");
        }
    }

    #[tokio::test]
    async fn fastbots_blog_crawl_smart() {
        if !run_live_tests() {
            eprintln!("SKIP: set RUN_LIVE_TESTS=1 to run");
            return;
        }

        let _ = env_logger::try_init();

        let result = tokio::time::timeout(Duration::from_secs(90), fetch_smart(URL)).await;
        assert!(result.is_ok(), "crawl_smart() should not timeout");

        let page = result.unwrap();
        assert!(
            page.is_some(),
            "crawl_smart() should return at least one page"
        );

        let page = page.unwrap();
        let html = page.get_html();
        let status = page.status_code.as_u16();
        eprintln!("crawl_smart(): {} bytes, status={}", html.len(), status);
        assert!(
            html.len() > 1000,
            "crawl_smart() HTML too small: {} bytes (status={})",
            html.len(),
            status
        );
    }

    #[tokio::test]
    async fn fastbots_blog_smart_matches_chrome() {
        if !run_live_tests() {
            eprintln!("SKIP: set RUN_LIVE_TESTS=1 to run");
            return;
        }

        let _ = env_logger::try_init();

        // --- Chrome rendering ---
        eprintln!("Fetching via crawl() (chrome)...");
        let chrome_result = tokio::time::timeout(Duration::from_secs(90), fetch_chrome(URL)).await;
        assert!(chrome_result.is_ok(), "crawl() should not timeout");

        let chrome_page = chrome_result.unwrap();
        if chrome_page.is_none() {
            eprintln!("SKIP: crawl() returned no page (chrome unavailable)");
            return;
        }
        let chrome_page = chrome_page.unwrap();
        let chrome_html = chrome_page.get_html();
        let chrome_len = chrome_html.len();

        eprintln!(
            "crawl()      : {} bytes, status={}",
            chrome_len,
            chrome_page.status_code.as_u16()
        );

        if chrome_len == 0 {
            eprintln!("SKIP: crawl() returned empty content");
            return;
        }

        // --- Smart mode ---
        eprintln!("Fetching via crawl_smart()...");
        let smart_result = tokio::time::timeout(Duration::from_secs(90), fetch_smart(URL)).await;
        assert!(smart_result.is_ok(), "crawl_smart() should not timeout");

        let smart_page = smart_result.unwrap();
        assert!(
            smart_page.is_some(),
            "crawl_smart() should return at least one page"
        );
        let smart_page = smart_page.unwrap();
        let smart_html = smart_page.get_html();
        let smart_len = smart_html.len();

        eprintln!(
            "crawl_smart(): {} bytes, status={}",
            smart_len,
            smart_page.status_code.as_u16()
        );

        // --- Comparison ---
        let chrome_tokens = text_tokens(&chrome_html);
        let smart_tokens = text_tokens(&smart_html);

        let overlap = chrome_tokens.intersection(&smart_tokens).count();
        let chrome_token_count = chrome_tokens.len();
        let overlap_pct = if chrome_token_count > 0 {
            (overlap as f64 / chrome_token_count as f64) * 100.0
        } else {
            0.0
        };

        let size_ratio = if chrome_len > 0 {
            smart_len as f64 / chrome_len as f64
        } else {
            0.0
        };

        eprintln!("=== fastbots.ai/blog: crawl vs crawl_smart ===");
        eprintln!("Chrome: {} bytes | Smart: {} bytes", chrome_len, smart_len);
        eprintln!("Size ratio  (smart/chrome): {:.2}", size_ratio);
        eprintln!(
            "Token overlap: {}/{} ({:.1}%)",
            overlap, chrome_token_count, overlap_pct
        );

        // Both should return substantial HTML content
        assert!(
            chrome_len > 1000,
            "crawl() HTML too small: {} bytes",
            chrome_len
        );
        assert!(
            smart_len > 1000,
            "crawl_smart() HTML too small: {} bytes",
            smart_len
        );

        // Smart should be at least 50% of chrome size
        assert!(
            size_ratio > 0.5,
            "crawl_smart() content too small vs crawl(): {:.2}x ({} vs {} bytes)",
            size_ratio,
            smart_len,
            chrome_len
        );

        // Text overlap: at least 50% of chrome tokens present in smart
        assert!(
            overlap_pct > 50.0,
            "Text overlap too low: {:.1}% (expected >50%)",
            overlap_pct
        );

        eprintln!("PASS: crawl_smart() content is comparable to crawl()");
    }
}
