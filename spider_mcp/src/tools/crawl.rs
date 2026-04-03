use rmcp::schemars;
use serde::Deserialize;
use serde_json::json;
use spider::tokio;
use spider::website::Website;
use spider_transformations::transformation::content::{
    transform_content_input, ReturnFormat, TransformConfig, TransformInput,
};
use std::sync::Arc;
use std::time::Instant;

use crate::state::{CrawlPageResult, CrawlSession, CrawlSessionStatus, SharedState};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct CrawlParams {
    /// The starting URL to crawl
    pub url: String,
    /// Maximum number of pages to crawl (default: 25)
    pub limit: Option<u32>,
    /// Maximum crawl depth (default: 25)
    pub depth: Option<usize>,
    /// Output format: raw, markdown, text, or xml (default: markdown)
    pub return_format: Option<String>,
    /// Honor robots.txt (default: true)
    pub respect_robots_txt: Option<bool>,
    /// Include subdomains (default: false)
    pub subdomains: Option<bool>,
    /// Use Chrome for JavaScript rendering
    pub headless: Option<bool>,
    /// Polite crawl delay in milliseconds
    pub delay_ms: Option<u64>,
    /// URL patterns to skip
    pub blacklist_urls: Option<Vec<String>>,
    /// Only crawl matching URL patterns
    pub whitelist_urls: Option<Vec<String>>,
    /// Additional domains to follow
    pub external_domains: Option<Vec<String>>,
    /// Proxy URL
    pub proxy: Option<String>,
    /// Custom User-Agent string
    pub user_agent: Option<String>,
}

/// Threshold: crawls at or below this limit run inline; above run in background.
const INLINE_LIMIT: u32 = 10;

pub async fn run(params: CrawlParams, state: Arc<SharedState>) -> Result<String, String> {
    let url = if params.url.starts_with("http") {
        params.url.clone()
    } else {
        format!("https://{}", params.url)
    };

    let limit = params.limit.unwrap_or(25);
    let mut website = Website::new(&url);
    super::apply_spider_cloud(&mut website);

    website
        .with_respect_robots_txt(params.respect_robots_txt.unwrap_or(true))
        .with_subdomains(params.subdomains.unwrap_or(false))
        .with_limit(limit);

    if let Some(depth) = params.depth {
        website.with_depth(depth);
    }
    if let Some(delay) = params.delay_ms {
        website.with_delay(delay);
    }
    if let Some(agent) = &params.user_agent {
        website.with_user_agent(Some(agent));
    }
    if let Some(proxy) = &params.proxy {
        if !proxy.is_empty() {
            website.with_proxies(Some(vec![proxy.clone()]));
        }
    }
    if let Some(blacklist) = &params.blacklist_urls {
        website.with_blacklist_url(Some(
            blacklist
                .iter()
                .map(|s| s.as_str().into())
                .collect::<Vec<spider::compact_str::CompactString>>(),
        ));
    }
    if let Some(whitelist) = &params.whitelist_urls {
        website.with_whitelist_url(Some(
            whitelist
                .iter()
                .map(|s| s.as_str().into())
                .collect::<Vec<spider::compact_str::CompactString>>(),
        ));
    }
    if let Some(domains) = params.external_domains.clone() {
        website.with_external_domains(Some(domains.into_iter()));
    }

    website.configuration.return_page_links = true;

    let format_str = params.return_format.as_deref().unwrap_or("markdown");
    let return_format = ReturnFormat::from_str(format_str);

    let mut website = website.build().map_err(|_| "Invalid URL".to_string())?;
    let mut rx = website.subscribe(16);

    let use_headless = params.headless.unwrap_or(false);

    tokio::spawn(async move {
        #[cfg(feature = "chrome")]
        {
            if use_headless {
                website.crawl().await;
            } else {
                website.crawl_raw().await;
            }
        }
        #[cfg(not(feature = "chrome"))]
        {
            let _ = use_headless;
            website.crawl().await;
        }
    });

    if limit <= INLINE_LIMIT {
        let transform_conf = TransformConfig {
            return_format,
            ..Default::default()
        };

        let mut pages = Vec::new();

        while let Ok(page) = rx.recv().await {
            let input = TransformInput {
                url: page.get_url_parsed_ref().as_ref(),
                content: page.get_html_bytes_u8(),
                screenshot_bytes: None,
                encoding: None,
                selector_config: None,
                ignore_tags: None,
            };
            let content = transform_content_input(input, &transform_conf);
            let links: Vec<String> = page
                .page_links
                .as_ref()
                .map(|s| s.iter().map(|l| l.inner().to_string()).collect())
                .unwrap_or_default();

            pages.push(json!({
                "url": page.get_url(),
                "status_code": page.status_code.as_u16(),
                "content": content,
                "links": links,
            }));
        }

        serde_json::to_string_pretty(&json!({ "pages": pages })).map_err(|e| e.to_string())
    } else {
        let crawl_id = uuid::Uuid::new_v4().to_string();
        state.sessions.insert(
            crawl_id.clone(),
            CrawlSession {
                status: CrawlSessionStatus::Running,
                pages: Vec::new(),
                started_at: Instant::now(),
            },
        );

        let state2 = state.clone();
        let id2 = crawl_id.clone();

        tokio::spawn(async move {
            let transform_conf = TransformConfig {
                return_format,
                ..Default::default()
            };

            while let Ok(page) = rx.recv().await {
                let input = TransformInput {
                    url: page.get_url_parsed_ref().as_ref(),
                    content: page.get_html_bytes_u8(),
                    screenshot_bytes: None,
                    encoding: None,
                    selector_config: None,
                    ignore_tags: None,
                };
                let content = transform_content_input(input, &transform_conf);
                let links: Vec<String> = page
                    .page_links
                    .as_ref()
                    .map(|s| s.iter().map(|l| l.inner().to_string()).collect())
                    .unwrap_or_default();

                if let Some(mut session) = state2.sessions.get_mut(&id2) {
                    session.pages.push(CrawlPageResult {
                        url: page.get_url().to_string(),
                        content,
                        status_code: page.status_code.as_u16(),
                        links,
                    });
                }
            }

            if let Some(mut session) = state2.sessions.get_mut(&id2) {
                session.status = CrawlSessionStatus::Complete;
            }
        });

        serde_json::to_string_pretty(&json!({
            "crawl_id": crawl_id,
            "status": "running",
            "message": format!("Crawl started. Use spider_crawl_status tool or read resource crawl://{crawl_id}/summary to check progress."),
        }))
        .map_err(|e| e.to_string())
    }
}
