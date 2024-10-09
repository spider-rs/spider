//! `OPENAI_API_KEY=$MYAPI_KEY cargo run --example openai_extra --features openai`
extern crate spider;

use spider::configuration::{GPTConfigs, WaitForIdleNetwork};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::website::Website;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let _ = tokio::fs::create_dir_all("./storage/").await;

    let screenshot_params =
        spider::configuration::ScreenshotParams::new(Default::default(), Some(true), Some(false));
    let screenshot_config =
        spider::configuration::ScreenShotConfig::new(screenshot_params, true, true, None);

    let mut gpt_config: GPTConfigs = GPTConfigs::new_multi(
        "gpt-4o",
        vec!["Search for Movies", "Extract the hrefs found."],
        3000,
    );

    gpt_config.screenshot = false;
    gpt_config.set_extra(true);

    let mut website: Website = Website::new("https://www.bing.com")
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(30)))))
        .with_screenshot(Some(screenshot_config))
        .with_limit(1)
        .with_openai(Some(gpt_config))
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("{}\n{:?}", page.get_url(), page.extra_ai_data);
        }
    });

    let start = crate::tokio::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();
    let links = website.get_links();

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}
