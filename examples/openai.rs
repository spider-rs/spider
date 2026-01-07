//! `OPENAI_API_KEY=$MYAPI_KEY cargo run --example openai --features openai`
extern crate spider;

use std::time::Duration;

use spider::configuration::{GPTConfigs, WaitForIdleNetwork};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::{HashMap, HashSet};
use spider::website::Website;
use spider::{tokio, CaseInsensitiveString};

#[tokio::main]
async fn main() {
    let _ = tokio::fs::create_dir_all("./storage/").await;
    let screenshot_params =
        spider::configuration::ScreenshotParams::new(Default::default(), Some(true), Some(false));
    let screenshot_config =
        spider::configuration::ScreenShotConfig::new(screenshot_params, true, true, None);

    let website_url = "https://www.google.com";

    let mut gpt_config = GPTConfigs::default();
    let prompt_url_map = HashMap::from([
        (
            CaseInsensitiveString::new(website_url),
            Box::new(GPTConfigs::new(
                "gpt-4o-2024-05-13",
                "Search for Movies",
                500,
            )),
        ),
        (
            CaseInsensitiveString::new(
                &((website_url.to_owned()) + "/search/howsearchworks/?fg=1"),
            ),
            Box::new(GPTConfigs::new(
                "gpt-4o-2024-05-13",
                "Change the background blue",
                500,
            )),
        ),
    ]);

    gpt_config.prompt_url_map = Some(Box::new(prompt_url_map));

    let mut website: Website = Website::new(website_url)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(30)))))
        .with_screenshot(Some(screenshot_config))
        .with_limit(2)
        .with_stealth(true)
        .with_fingerprint(true)
        // .with_chrome_connection(Some("http://127.0.0.1:9222/json/version".into()))
        .with_openai(Some(gpt_config))
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();

    website.set_extra_links(HashSet::from([(website_url.to_owned()
        + "/search/howsearchworks/?fg=1")
        .into()]));

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("{}\n{}", page.get_url(), page.get_html());
        }
    });

    let start = crate::tokio::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();
    let links = website.get_all_links_visited().await;

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}
