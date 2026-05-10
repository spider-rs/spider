//! End-to-end repro for smart-mode chrome fallback on transport-level HTTP failures.
//!
//! `https://www.buildinghub.io/` ships a Let's Encrypt cert whose
//! `subjectAltName` only contains `DNS:buildinghub.io` (no `www`). The bare
//! reqwest path therefore fails the TLS handshake with
//! `invalid peer certificate: NotValidForName` *before* any HTTP — meaning
//! the seed page returns empty + 5xx and the smart `is_empty()` extraction
//! short-circuits, never running the JS-detection branch that upgrades to
//! chrome. With default `retry=0` the surrounding retry loop also doesn't
//! run, so smart degrades to plain HTTP-only and the user sees a hard
//! failure even though the server returns a perfectly clean
//! `301 Location: https://buildinghub.io/` once you get past the cert.
//!
//! Chrome's `--ignore-certificate-errors` lets it proceed past the SAN
//! mismatch and follow the 301 to the bare host (where the cert IS valid),
//! returning a normal 200 page. The fix wires a single chrome attempt at
//! the end of `crawl_establish_smart` (and its sibling in
//! `crawl_concurrent_smart`) when HTTP produced no content and the loop
//! never invoked chrome itself.
//!
//! Run: RUN_LIVE_TESTS=1 cargo test -p spider --test buildinghub_smart_cert_repro \
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

    #[tokio::test]
    #[ignore]
    async fn smart_recovers_from_cert_san_mismatch_via_chrome_fallback() {
        if !run_live_tests() {
            eprintln!("SKIP: set RUN_LIVE_TESTS=1");
            return;
        }
        let _ = env_logger::try_init();

        let url = "https://www.buildinghub.io/";
        let mut w = Website::new(url);
        // No retry, no caching — exactly the user's repro shape from the
        // spider cloud `request: smart, cache: false` ticket.
        w.with_limit(1)
            .with_caching(false)
            .with_request_timeout(Some(Duration::from_secs(30)))
            .with_crawl_timeout(Some(Duration::from_secs(60)))
            .with_respect_robots_txt(false);

        let mut rx = w.subscribe(8);
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
        let outcome = tokio::time::timeout(Duration::from_secs(90), w.crawl_smart()).await;
        let elapsed = start.elapsed();
        w.unsubscribe();
        let pages = collector.await.unwrap_or_default();

        assert!(outcome.is_ok(), "crawl_smart hung past 90s");
        assert!(
            elapsed < Duration::from_secs(60),
            "smart crawl too slow ({:?})",
            elapsed
        );

        let seed = pages
            .iter()
            .find(|p| p.get_url() == url)
            .expect("seed page must be in subscriber stream");

        // Pre-fix: status_code was 503 (or 524 behind a proxy that surfaces
        // tunnel timeouts) and the body was empty because reqwest refused
        // the cert and `smart_links` short-circuited on `is_empty()`.
        assert_eq!(
            seed.status_code.as_u16(),
            200,
            "seed page must come back 200 via chrome fallback (got {})",
            seed.status_code
        );
        assert!(
            seed.get_html_bytes_u8().len() > 10_000,
            "seed body too small for the rendered homepage: {} bytes",
            seed.get_html_bytes_u8().len()
        );
    }
}
