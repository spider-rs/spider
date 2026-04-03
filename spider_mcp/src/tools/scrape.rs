use rmcp::schemars;
use serde::Deserialize;
use serde_json::json;
use spider::tokio;
use spider::website::Website;
use spider_transformations::transformation::content::{
    transform_content_input, ReturnFormat, TransformConfig, TransformInput,
};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ScrapeParams {
    /// The URL to scrape
    pub url: String,
    /// Output format: raw, markdown, text, or xml (default: markdown)
    pub return_format: Option<String>,
    /// Use Chrome for JavaScript rendering (requires chrome feature)
    pub headless: Option<bool>,
    /// CSS selector to wait for before extraction
    pub wait_for: Option<String>,
    /// Wait N milliseconds after page load
    pub wait_for_delay_ms: Option<u64>,
    /// Wait for network to become idle
    pub wait_for_idle_network: Option<bool>,
    /// Custom User-Agent string
    pub user_agent: Option<String>,
    /// Cookie string (e.g. "key=val; key2=val2")
    pub cookie: Option<String>,
    /// Proxy URL
    pub proxy: Option<String>,
}

pub async fn run(params: ScrapeParams) -> Result<String, String> {
    let url = if params.url.starts_with("http") {
        params.url.clone()
    } else {
        format!("https://{}", params.url)
    };

    let mut website = Website::new(&url);
    super::apply_spider_cloud(&mut website);

    if let Some(agent) = &params.user_agent {
        website.with_user_agent(Some(agent));
    }
    if let Some(cookie) = &params.cookie {
        website.with_cookies(cookie);
    }
    if let Some(proxy) = &params.proxy {
        if !proxy.is_empty() {
            website.with_proxies(Some(vec![proxy.clone()]));
        }
    }

    super::apply_wait_options(
        &mut website,
        &params.wait_for,
        params.wait_for_delay_ms,
        params.wait_for_idle_network,
    );

    website.configuration.return_page_links = true;
    website.with_limit(1);

    let mut website = website.build().map_err(|_| "Invalid URL".to_string())?;

    let mut rx = website.subscribe(0);

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

    let format_str = params.return_format.as_deref().unwrap_or("markdown");
    let transform_conf = TransformConfig {
        return_format: ReturnFormat::from_str(format_str),
        ..Default::default()
    };

    let mut results = Vec::new();

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

        results.push(json!({
            "url": page.get_url(),
            "status_code": page.status_code.as_u16(),
            "content": content,
            "links": links,
        }));
    }

    if results.is_empty() {
        return Err(format!("No content returned for {}", params.url));
    }

    if results.len() == 1 {
        serde_json::to_string_pretty(&results[0]).map_err(|e| e.to_string())
    } else {
        serde_json::to_string_pretty(&results).map_err(|e| e.to_string())
    }
}
