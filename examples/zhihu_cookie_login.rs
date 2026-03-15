//! Crawl a logged-in page with session cookies captured from your browser.
//!
//! Usage:
//! COOKIE='z_c0=...; d_c0=...' \
//! cargo run --example zhihu_cookie_login \
//!   --features="spider/sync spider/cookies spider/chrome" \
//!   -- 'https://www.zhihu.com/question/123456789'
//!
//! Optional environment variables:
//! - TARGET_URL: fallback target when no CLI arg is provided
//! - OUTPUT_HTML: where to save the rendered HTML
//! - OUTPUT_JSON: where to save extracted text blocks
//! - TITLE_SELECTORS: comma-separated CSS/XPath selectors
//! - CONTENT_SELECTORS: comma-separated CSS/XPath selectors
//! - CHROME_CONNECTION_URL: remote CDP endpoint like http://HOST:9222/json/version
//! - COOKIE: optional cookie string; not required when remote browser is already logged in

extern crate env_logger;
extern crate spider;

use env_logger::Env;
use spider::client::header::{HeaderMap, HeaderValue, ACCEPT_LANGUAGE, REFERER, USER_AGENT};
use spider::configuration::{WaitForIdleNetwork, WaitForSelector};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::{HashMap, HashSet};
use spider::page::Page;
use spider::tokio;
use spider::website::Website;
use spider_utils::build_selectors;
use std::env;
use std::io::{Error, ErrorKind, Result};
use std::time::Duration;

fn split_selectors(value: &str) -> HashSet<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn extraction_selectors() -> spider_utils::DocumentSelectors<String> {
    let title = env::var("TITLE_SELECTORS").unwrap_or_else(|_| "title,h1".to_string());
    let content = env::var("CONTENT_SELECTORS").unwrap_or_else(|_| {
        "main,article,.Question-main,.Post-RichTextContainer,.RichContent,.RichText".to_string()
    });

    let mut selector_map: HashMap<String, HashSet<String>> = HashMap::new();
    selector_map.insert("title".to_string(), split_selectors(&title));
    selector_map.insert("content".to_string(), split_selectors(&content));

    build_selectors(selector_map)
}

fn json_error(err: serde_json::Error) -> Error {
    Error::other(err)
}

#[tokio::main]
async fn main() -> Result<()> {
    let env = Env::default()
        .filter_or("RUST_LOG", "info")
        .write_style_or("RUST_LOG_STYLE", "always");
    env_logger::init_from_env(env);

    let cookie = env::var("COOKIE").ok();
    let chrome_connection_url = env::var("CHROME_CONNECTION_URL").ok();
    let target = env::args()
        .nth(1)
        .or_else(|| env::var("TARGET_URL").ok())
        .unwrap_or_else(|| "https://www.zhihu.com/".to_string());
    let output_html = env::var("OUTPUT_HTML").unwrap_or_else(|_| "zhihu.html".to_string());
    let output_json =
        env::var("OUTPUT_JSON").unwrap_or_else(|_| "zhihu_extracted.json".to_string());

    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36",
        ),
    );
    headers.insert(REFERER, HeaderValue::from_static("https://www.zhihu.com/"));
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
    );

    let mut website: Website = Website::new(&target)
        .with_limit(1)
        .with_headers(Some(headers))
        .with_wait_for_idle_network0(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(3)))))
        .with_wait_for_idle_dom(Some(WaitForSelector::new(
            Some(Duration::from_millis(500)),
            "body".into(),
        )))
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true);

    if let Some(cookie) = cookie.as_deref() {
        website.with_cookies(cookie);
    }

    if let Some(chrome_connection_url) = chrome_connection_url {
        website.with_chrome_connection(Some(chrome_connection_url));
    }

    let mut website = website
        .build()
        .map_err(|_| Error::other("failed to build website config"))?;

    let mut rx = website
        .subscribe(16)
        .ok_or_else(|| Error::other("failed to subscribe to page stream"))?;

    let collector = tokio::spawn(async move {
        let mut pages: Vec<Page> = Vec::new();

        while let Ok(page) = rx.recv().await {
            pages.push(page);
        }

        pages
    });

    website.crawl().await;
    website.unsubscribe();

    let pages = collector
        .await
        .map_err(|err| Error::other(format!("collector task failed: {err}")))?;

    let page = pages
        .first()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "no page was captured"))?;

    let html = page.get_html();
    std::fs::write(&output_html, &html)?;

    let extracted =
        spider_utils::css_query_select_map_streamed(&html, &extraction_selectors()).await;
    std::fs::write(
        &output_json,
        serde_json::to_vec_pretty(&extracted).map_err(json_error)?,
    )?;

    println!("Captured URL: {}", page.get_url());
    println!("Final URL: {}", page.get_url_final());
    println!("HTML bytes: {}", page.get_html_bytes_u8().len());
    println!("Saved HTML to: {}", output_html);
    println!("Saved extracted content to: {}", output_json);
    println!(
        "{}",
        serde_json::to_string_pretty(&extracted).map_err(json_error)?
    );

    Ok(())
}
