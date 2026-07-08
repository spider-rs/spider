//! End-to-end offline test for the in-process `HttpFetchEngine` seam.
//!
//! A mock engine intercepts every fetch (`should_fetch` accepts all URLs),
//! so `crawl_raw` runs with zero network I/O. This proves the raw-path
//! wiring reaches the engine, that spider builds a `Page` from the
//! engine's `EngineResponse`, and — the key Point B parity check — that
//! link extraction runs on the engine-served HTML so the crawl discovers
//! and follows links exactly as it would on the reqwest path.

use spider::fetch_engine::{EngineError, EngineRequest, EngineResponse, HttpFetchEngine};
use spider::website::Website;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const SEED: &str = "https://engine.test/";
const CHILD: &str = "https://engine.test/child";

struct MockEngine {
    hits: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl HttpFetchEngine for MockEngine {
    async fn fetch(&self, req: EngineRequest<'_>) -> Result<EngineResponse, EngineError> {
        self.hits.fetch_add(1, Ordering::SeqCst);

        // Seed links to CHILD; everything else is a leaf. Same body shape
        // keeps the crawl bounded by the link graph.
        let body = if req.url.starts_with(CHILD) {
            "<html><head><title>leaf</title></head><body>leaf</body></html>".to_string()
        } else {
            format!(
                r#"<html><head><title>seed</title></head><body><a href="{CHILD}">child</a></body></html>"#
            )
        };

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "text/html; charset=utf-8".parse().unwrap(),
        );

        Ok(EngineResponse {
            status_code: reqwest::StatusCode::OK,
            final_url: Some(req.url.to_string()),
            headers,
            body: body.into_bytes(),
            served: true,
            ..Default::default()
        })
    }
}

#[tokio::test]
async fn engine_serves_raw_crawl_and_extracts_links() {
    let hits = Arc::new(AtomicUsize::new(0));

    let mut website = Website::new(SEED);
    website
        .with_fetch_engine(MockEngine { hits: hits.clone() })
        .with_respect_robots_txt(false)
        .with_limit(10);

    website.crawl_raw().await;

    // The engine served every fetch (no network). Two hits means the seed
    // was fetched AND the child link — discovered by extracting links from
    // the engine-served HTML — was followed. One hit would mean link
    // extraction on the engine path failed.
    assert!(
        hits.load(Ordering::SeqCst) >= 2,
        "engine should have served the seed and the extracted child (hits={})",
        hits.load(Ordering::SeqCst)
    );

    // Both URLs were visited via the engine path.
    let links = website.get_links();
    let has_child = links.iter().any(|l| l.as_ref() == CHILD);
    assert!(
        has_child,
        "child link should have been extracted from engine HTML and visited: {:?}",
        links
            .iter()
            .map(|l| l.as_ref().to_string())
            .collect::<Vec<_>>()
    );
}

/// When `should_fetch` declines a URL, spider must fall through to its
/// normal (reqwest) path — the engine is not consulted for the body.
#[tokio::test]
async fn engine_declines_falls_through() {
    struct Declining {
        hits: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl HttpFetchEngine for Declining {
        async fn fetch(&self, _req: EngineRequest<'_>) -> Result<EngineResponse, EngineError> {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Ok(EngineResponse::default())
        }
        fn should_fetch(&self, _url: &str) -> bool {
            false
        }
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let mut website = Website::new("https://nonexistent.invalid/");
    website
        .with_fetch_engine(Declining { hits: hits.clone() })
        .with_respect_robots_txt(false)
        .with_limit(1);
    website.crawl_raw().await;

    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "engine must not be consulted when should_fetch returns false"
    );
}
