//! `OPENAI_API_KEY=$MYAPI_KEY cargo run --example openai_multi --features openai`
extern crate spider;

use spider::configuration::{GPTConfigs, WaitForIdleNetwork};
use spider::tokio;
use spider::website::Website;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let _ = tokio::fs::create_dir_all("./storage/").await;

    let screenshot_params =
        spider::configuration::ScreenshotParams::new(Default::default(), Some(true), Some(false));
    // params that handle the way to take screenshots
    let screenshot_config =
        spider::configuration::ScreenShotConfig::new(screenshot_params, true, true, None);

    let gpt_config: GPTConfigs = GPTConfigs::new_multi(
        "gpt-4-1106-preview",
        vec![
            "Search for Movies",
            "Click on the first result movie result",
        ],
        500,
    );

    let mut website: Website = Website::new("https://www.google.com")
        .with_chrome_intercept(true, true)
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(30)))))
        .with_screenshot(Some(screenshot_config))
        .with_limit(1)
        .with_openai(Some(gpt_config))
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("{}\n{}", page.get_url(), page.get_html());
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
