pub mod crawl;
pub mod links;
pub mod scrape;
pub mod transform;

use spider::features::chrome_common::{WaitForDelay, WaitForIdleNetwork, WaitForSelector};
use spider::website::Website;
use std::time::Duration;

/// Apply common wait-for options to a Website builder.
pub fn apply_wait_options(
    website: &mut Website,
    wait_for: &Option<String>,
    wait_for_delay_ms: Option<u64>,
    wait_for_idle_network: Option<bool>,
) {
    if let Some(selector) = wait_for {
        website.with_wait_for_selector(Some(WaitForSelector::new(
            Some(Duration::from_secs(60)),
            selector.clone(),
        )));
    }
    if let Some(ms) = wait_for_delay_ms {
        website.with_wait_for_delay(Some(WaitForDelay::new(Some(Duration::from_millis(ms)))));
    }
    if wait_for_idle_network == Some(true) {
        website.with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(
            Duration::from_millis(500),
        ))));
    }
}

/// Apply Spider Cloud API key if SPIDER_API_KEY env var is set.
pub fn apply_spider_cloud(website: &mut Website) {
    if let Ok(api_key) = std::env::var("SPIDER_API_KEY") {
        if !api_key.is_empty() {
            website.with_spider_cloud(&api_key);
        }
    }
}
