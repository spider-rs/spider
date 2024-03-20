//! `OPENAI_API_KEY=$MYAPI_KEY cargo run --example openai --features openai`
extern crate spider;

use std::time::Duration;

use spider::configuration::{GPTConfigs, WaitForIdleNetwork};
use spider::hashbrown::{HashMap, HashSet};
use spider::website::Website;
use spider::{tokio, CaseInsensitiveString};

#[tokio::main]
async fn main() {
    let _ = tokio::fs::create_dir_all("./storage/").await;

    let screenshot_params =
        spider::configuration::ScreenshotParams::new(Default::default(), Some(true), Some(false));
    // params that handle the way to take screenshots
    let screenshot_config =
        spider::configuration::ScreenShotConfig::new(screenshot_params, true, true, None);

    let mut gpt_config = GPTConfigs::new("gpt-4-1106-preview", "Search for Movies", 500);

    let mut prompt_url_map = HashMap::new();

    prompt_url_map.insert(
        CaseInsensitiveString::new("https://www.google.com/search/howsearchworks/?fg=1"),
        GPTConfigs::new("gpt-4-1106-preview", "Change the background blue", 500),
    );

    gpt_config.prompt_url_map = Some(prompt_url_map);

    let mut website: Website = Website::new("https://www.google.com")
        .with_chrome_intercept(true, true)
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(30)))))
        .with_screenshot(Some(screenshot_config))
        .with_limit(2)
        .with_openai(Some(gpt_config))
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(16).unwrap();

    website.set_extra_links(HashSet::from([
        "https://www.google.com/search/howsearchworks/?fg=1".into(),
    ]));

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
