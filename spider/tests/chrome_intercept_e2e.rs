//! End-to-end integration tests that connect to a real Chrome instance and
//! verify that goto_with_html_once correctly injects HTML without deadlocking.
//!
//! Requires: Chrome running with --remote-debugging-port=9222
//! Run: CHROME_URL=ws://127.0.0.1:9222 cargo test -p spider --test chrome_intercept_e2e --features "chrome chrome_intercept smart"

#[cfg(feature = "chrome")]
mod e2e {
    use spider::tokio;
    use spider::website::Website;
    use std::time::Duration;

    /// Get the full devtools WS URL from a local Chrome instance.
    /// `Browser::connect_with_config` requires the full path (e.g.
    /// ws://host:port/devtools/browser/UUID), not just ws://host:port.
    async fn chrome_url() -> Option<String> {
        if std::net::TcpStream::connect("127.0.0.1:9222").is_err() {
            return None;
        }
        let resp = spider::reqwest::get("http://127.0.0.1:9222/json/version")
            .await
            .ok()?;
        let json: serde_json::Value = resp.json().await.ok()?;
        json["webSocketDebuggerUrl"].as_str().map(String::from)
    }

    /// Basic Chrome crawl without chrome_intercept features — the common path.
    #[tokio::test]
    async fn basic_chrome_crawl() {
        let Some(ws) = chrome_url().await else {
            eprintln!("SKIP: no Chrome at port 9222");
            return;
        };

        let mut website = Website::new("https://example.com")
            .with_chrome_connection(Some(ws.into()))
            .with_limit(1)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .build()
            .unwrap();

        let mut rx = website.subscribe(16);
        let collector = tokio::spawn(async move {
            let mut pages = vec![];
            while let Ok(page) = rx.recv().await {
                pages.push((page.get_url().to_string(), page.get_html().len()));
            }
            pages
        });

        let start = std::time::Instant::now();
        website.crawl().await;
        website.unsubscribe();
        let elapsed = start.elapsed();

        let pages = collector.await.unwrap();
        eprintln!("basic_chrome_crawl: {} pages in {:?}", pages.len(), elapsed);

        assert!(!pages.is_empty(), "Should have crawled at least 1 page");
        assert!(pages[0].1 > 100, "Page should have content ({} bytes)", pages[0].1);
        assert!(elapsed < Duration::from_secs(45), "Should not deadlock ({:?})", elapsed);
    }

    /// Chrome crawl with chrome_intercept — resource blocking active.
    #[cfg(feature = "chrome_intercept")]
    #[tokio::test]
    async fn chrome_intercept_crawl() {
        let Some(ws) = chrome_url().await else {
            eprintln!("SKIP: no Chrome at port 9222");
            return;
        };

        let mut website = Website::new("https://example.com")
            .with_chrome_connection(Some(ws.into()))
            .with_limit(1)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .build()
            .unwrap();

        let mut rx = website.subscribe(16);
        let collector = tokio::spawn(async move {
            let mut pages = vec![];
            while let Ok(page) = rx.recv().await {
                pages.push((page.get_url().to_string(), page.get_html().len()));
            }
            pages
        });

        let start = std::time::Instant::now();
        website.crawl().await;
        website.unsubscribe();
        let elapsed = start.elapsed();

        let pages = collector.await.unwrap();
        eprintln!("chrome_intercept_crawl: {} pages in {:?}", pages.len(), elapsed);

        assert!(!pages.is_empty(), "Should have crawled at least 1 page");
        assert!(pages[0].1 > 100, "Page should have content ({} bytes)", pages[0].1);
        assert!(elapsed < Duration::from_secs(45), "Should not deadlock ({:?})", elapsed);
    }

    /// Multiple concurrent pages with chrome_intercept — most likely to trigger blocking.
    #[cfg(feature = "chrome_intercept")]
    #[tokio::test]
    async fn chrome_intercept_concurrent() {
        let Some(ws) = chrome_url().await else {
            eprintln!("SKIP: no Chrome at port 9222");
            return;
        };

        let mut website = Website::new("https://example.com")
            .with_chrome_connection(Some(ws.into()))
            .with_depth(1)
            .with_limit(4)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .build()
            .unwrap();

        let mut rx = website.subscribe(32);
        let collector = tokio::spawn(async move {
            let mut count = 0usize;
            while let Ok(_page) = rx.recv().await {
                count += 1;
            }
            count
        });

        let start = std::time::Instant::now();
        website.crawl().await;
        website.unsubscribe();
        let elapsed = start.elapsed();

        let count = collector.await.unwrap();
        eprintln!("chrome_intercept_concurrent: {} pages in {:?}", count, elapsed);

        assert!(count >= 1, "Should have visited at least 1 page");
        assert!(elapsed < Duration::from_secs(60), "No deadlock ({:?})", elapsed);
    }

    /// Smart mode — HTTP first, Chrome upgrade for JS content.
    #[cfg(feature = "smart")]
    #[tokio::test]
    async fn smart_mode_crawl() {
        let Some(ws) = chrome_url().await else {
            eprintln!("SKIP: no Chrome at port 9222");
            return;
        };

        let mut website = Website::new("https://example.com")
            .with_chrome_connection(Some(ws.into()))
            .with_limit(1)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .build()
            .unwrap();

        let mut rx = website.subscribe(16);
        let collector = tokio::spawn(async move {
            let mut pages = vec![];
            while let Ok(page) = rx.recv().await {
                pages.push((page.get_url().to_string(), page.get_html().len()));
            }
            pages
        });

        let start = std::time::Instant::now();
        website.crawl_smart().await;
        website.unsubscribe();
        let elapsed = start.elapsed();

        let pages = collector.await.unwrap();
        eprintln!("smart_mode: {} pages in {:?}", pages.len(), elapsed);

        assert!(!pages.is_empty(), "Should have crawled at least 1 page");
        assert!(elapsed < Duration::from_secs(60), "No deadlock ({:?})", elapsed);
    }

    /// Verify the seeded content path (goto_with_html_once) doesn't deadlock.
    /// Crawl twice: first populates cache, second uses cached content.
    #[cfg(feature = "chrome_intercept")]
    #[tokio::test]
    async fn seeded_content_no_deadlock() {
        let Some(ws) = chrome_url().await else {
            eprintln!("SKIP: no Chrome at port 9222");
            return;
        };

        // First crawl — populates pages
        let mut website1 = Website::new("https://example.com")
            .with_chrome_connection(Some(ws.clone().into()))
            .with_limit(1)
            .with_caching(true)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .build()
            .unwrap();

        let mut rx1 = website1.subscribe(16);
        let c1 = tokio::spawn(async move {
            let mut n = 0usize;
            while let Ok(_) = rx1.recv().await { n += 1; }
            n
        });

        website1.crawl().await;
        website1.unsubscribe();
        let n1 = c1.await.unwrap();
        eprintln!("seeded_content first crawl: {} pages", n1);

        // Second crawl — should use cached content (goto_with_html_once path)
        let mut website2 = Website::new("https://example.com")
            .with_chrome_connection(Some(ws.into()))
            .with_limit(1)
            .with_caching(true)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .build()
            .unwrap();

        let mut rx2 = website2.subscribe(16);
        let c2 = tokio::spawn(async move {
            let mut n = 0usize;
            while let Ok(_) = rx2.recv().await { n += 1; }
            n
        });

        let start = std::time::Instant::now();
        website2.crawl().await;
        website2.unsubscribe();
        let elapsed = start.elapsed();
        let n2 = c2.await.unwrap();

        eprintln!("seeded_content second crawl: {} pages in {:?}", n2, elapsed);
        assert!(
            elapsed < Duration::from_secs(45),
            "Second crawl (cached path) deadlocked: {:?}",
            elapsed
        );
    }
}
