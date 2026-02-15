use spider::hashbrown::HashMap;
use spider::page::Page;
use spider::website::Website;
use std::env;
use std::time::Duration;

pub const BASE: &str = "https://www.crawler-test.com";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CRAWL_TIMEOUT: Duration = Duration::from_secs(60);

pub fn run_live_tests() -> bool {
    matches!(
        env::var("RUN_LIVE_TESTS")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Fetch a single page via HTTP (no browser).
pub async fn fetch_page_http(path: &str) -> Page {
    let url = format!("{}{}", BASE, path);
    let client = spider::reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(spider::reqwest::redirect::Policy::limited(10))
        .build()
        .expect("build http client");
    Page::new_page(&url, &client).await
}

/// Fetch a single page via HTTP with no redirect following.
pub async fn fetch_page_http_no_redirect(path: &str) -> Page {
    let url = format!("{}{}", BASE, path);
    let client = spider::reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(spider::reqwest::redirect::Policy::none())
        .build()
        .expect("build http client (no redirect)");
    Page::new_page(&url, &client).await
}

/// Crawl multiple pages via HTTP, collecting results lock-free through broadcast channels.
/// Uses tokio::join! + tokio::select! pattern (same as spider's scrape_raw).
pub async fn crawl_collect_http(path: &str, budget: u32, depth: usize) -> Vec<Page> {
    let url = format!("{}{}", BASE, path);
    let mut website = Website::new(&url);
    website
        .with_budget(Some(HashMap::from([("*", budget)])))
        .with_depth(depth)
        .with_request_timeout(Some(REQUEST_TIMEOUT))
        .with_crawl_timeout(Some(CRAWL_TIMEOUT));

    let mut w = website.clone();
    let mut rx = w.subscribe(16).expect("subscribe");
    let (done_tx, mut done_rx) = spider::tokio::sync::oneshot::channel::<()>();

    let crawl = async move {
        w.crawl_raw().await;
        w.unsubscribe();
        let _ = done_tx.send(());
    };

    let mut pages = Vec::new();
    let sub = async {
        loop {
            spider::tokio::select! {
                biased;
                _ = &mut done_rx => break,
                result = rx.recv() => {
                    if let Ok(page) = result {
                        pages.push(page);
                    } else {
                        break;
                    }
                }
            }
        }
    };

    spider::tokio::join!(sub, crawl);
    pages
}

/// Crawl and collect pages using smart mode.
#[cfg(feature = "smart")]
pub async fn crawl_collect_smart(path: &str, budget: u32, depth: usize) -> Vec<Page> {
    let url = format!("{}{}", BASE, path);
    let mut website = Website::new(&url);
    website
        .with_budget(Some(HashMap::from([("*", budget)])))
        .with_depth(depth)
        .with_request_timeout(Some(REQUEST_TIMEOUT))
        .with_crawl_timeout(Some(CRAWL_TIMEOUT));

    let mut w = website.clone();
    let mut rx = w.subscribe(16).expect("subscribe");
    let (done_tx, mut done_rx) = spider::tokio::sync::oneshot::channel::<()>();

    let crawl = async move {
        w.crawl_smart().await;
        w.unsubscribe();
        let _ = done_tx.send(());
    };

    let mut pages = Vec::new();
    let sub = async {
        loop {
            spider::tokio::select! {
                biased;
                _ = &mut done_rx => break,
                result = rx.recv() => {
                    if let Ok(page) = result {
                        pages.push(page);
                    } else {
                        break;
                    }
                }
            }
        }
    };

    spider::tokio::join!(sub, crawl);
    pages
}

/// Fetch a single page via Chrome using subscribe + crawl.
/// Uses the same tokio::join! + select! pattern as scrape_raw.
#[cfg(feature = "chrome")]
pub async fn fetch_page_chrome(path: &str) -> Option<Page> {
    let url = format!("{}{}", BASE, path);
    let mut website = Website::new(&url);
    website
        .with_limit(1)
        .with_depth(0)
        .with_request_timeout(Some(REQUEST_TIMEOUT))
        .with_crawl_timeout(Some(CRAWL_TIMEOUT));

    let mut w = website.clone();
    let mut rx = w.subscribe(4).expect("subscribe");
    let (done_tx, mut done_rx) = spider::tokio::sync::oneshot::channel::<()>();

    let crawl = async move {
        w.crawl().await;
        w.unsubscribe();
        let _ = done_tx.send(());
    };

    let mut page = None;
    let sub = async {
        loop {
            spider::tokio::select! {
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

    spider::tokio::join!(sub, crawl);
    page
}

/// Extract text content between specific HTML tags using simple string search.
pub fn extract_tag_content(html: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag.split_whitespace().next().unwrap_or(tag));
    let start = html.find(&open)?;
    let after_open = html[start..].find('>')?;
    let content_start = start + after_open + 1;
    let end = html[content_start..].find(&close)?;
    Some(html[content_start..content_start + end].trim().to_string())
}

/// Extract canonical URL from HTML.
pub fn extract_canonical(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let idx = lower.find("rel=\"canonical\"")?;
    let snippet_start = if idx > 200 { idx - 200 } else { 0 };
    let snippet_end = (idx + 200).min(html.len());
    let snippet = &html[snippet_start..snippet_end];
    let href_idx = snippet.to_lowercase().find("href=\"")?;
    let href_start = href_idx + 6;
    let href_end = snippet[href_start..].find('"')?;
    Some(snippet[href_start..href_start + href_end].to_string())
}

/// Extract meta description from HTML.
pub fn extract_meta_description(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let idx = lower.find("name=\"description\"")?;
    let snippet_start = if idx > 300 { idx - 300 } else { 0 };
    let snippet_end = (idx + 300).min(html.len());
    let snippet = &html[snippet_start..snippet_end];
    let lower_snippet = snippet.to_lowercase();
    let content_idx = lower_snippet.find("content=\"")?;
    let content_start = content_idx + 9;
    let content_end = snippet[content_start..].find('"')?;
    Some(snippet[content_start..content_start + content_end].to_string())
}

/// Extract page title from HTML.
pub fn extract_title(html: &str) -> Option<String> {
    extract_tag_content(html, "title")
}
