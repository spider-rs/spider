use rmcp::schemars;
use serde::Deserialize;
use serde_json::json;
use spider::tokio;
use spider::website::Website;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct LinksParams {
    /// The URL to extract links from
    pub url: String,
    /// Use Chrome for JavaScript rendering
    pub headless: Option<bool>,
    /// Include subdomain links (default: false)
    pub subdomains: Option<bool>,
}

pub async fn run(params: LinksParams) -> Result<String, String> {
    let url = if params.url.starts_with("http") {
        params.url.clone()
    } else {
        format!("https://{}", params.url)
    };

    let mut website = Website::new(&url);
    super::apply_spider_cloud(&mut website);

    website
        .with_subdomains(params.subdomains.unwrap_or(false))
        .with_limit(1);

    website.configuration.return_page_links = true;

    let mut website = website.build().map_err(|_| "Invalid URL".to_string())?;

    let mut rx = website
        .subscribe(0)
        .ok_or_else(|| "Subscribe failed (sync feature required)".to_string())?;

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

    if let Ok(page) = rx.recv().await {
        let links: Vec<String> = page
            .page_links
            .as_ref()
            .map(|s| s.iter().map(|l| l.inner().to_string()).collect())
            .unwrap_or_default();
        let count = links.len();

        serde_json::to_string_pretty(&json!({
            "url": page.get_url(),
            "links": links,
            "count": count,
        }))
        .map_err(|e| e.to_string())
    } else {
        Err(format!("No response for {}", params.url))
    }
}
