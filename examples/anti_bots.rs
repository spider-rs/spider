//! cargo run --example anti_bots --features="chrome chrome_intercept real_browser spider_utils/transformations"

use spider::configuration::{ChromeEventTracker, Fingerprint};
use spider::features::chrome_common::{
    RequestInterceptConfiguration, WaitForIdleNetwork, WaitForSelector,
};
use spider::features::chrome_viewport;
use spider::tokio;
use spider::tokio::io::AsyncWriteExt;
use spider::website::Website;
use std::fs::create_dir_all;
use std::time::Duration;

async fn crawl_website(url: &str) {
    let mut stdout: tokio::io::Stdout = tokio::io::stdout();
    let mut interception = RequestInterceptConfiguration::new(true);
    let mut tracker = ChromeEventTracker::default();

    interception.block_javascript = false;
    interception.block_stylesheets = false;
    interception.block_visuals = false;
    interception.block_ads = false;
    interception.block_analytics = false;

    tracker.responses = true;
    tracker.requests = true;

    let screenshot_params: spider::configuration::ScreenshotParams =
        spider::configuration::ScreenshotParams::new(Default::default(), Some(true), Some(false));
    let screenshot_config =
        spider::configuration::ScreenShotConfig::new(screenshot_params, true, true, None);

    let viewport = chrome_viewport::randomize_viewport(&chrome_viewport::DeviceType::Desktop);

    let mut website: Website = Website::new(url)
        .with_limit(1)
        .with_retry(0)
        .with_chrome_intercept(interception)
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_millis(500)))))
        .with_wait_for_idle_dom(Some(WaitForSelector::new(
            Some(Duration::from_millis(5000)),
            "body".into(),
        )))
        .with_screenshot(Some(screenshot_config))
        .with_block_assets(true)
        .with_viewport(Some(viewport))
        .with_user_agent(Some("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36"))
        .with_stealth(true)
        .with_return_page_links(true)
        .with_event_tracker(Some(tracker))
        .with_fingerprint_advanced(Fingerprint::None)
        .with_proxies(Some(vec!["http://localhost:8888".into()]))
        .with_chrome_connection(Some("http://127.0.0.1:9222/json/version".into()))
        .build()
        .unwrap();

    let mut rx2 = website.subscribe(16).unwrap();
    let mut g = website.subscribe_guard().unwrap();
    g.guard(true);

    let start = crate::tokio::time::Instant::now();

    let (links, _) = tokio::join!(
        async move {
            website.crawl().await;
            website.unsubscribe();
            website.get_all_links_visited().await
        },
        async move {
            while let Ok(page) = rx2.recv().await {
                let _ = stdout
                    .write_all(
                        format!(
                            "---- {}\nBytes transferred {:?}\nHTML Size {:?}\nLinks: {:?}\nRequests Sent {:?}\nStatus: {:?}\n",
                            page.get_url(),
                            page.bytes_transferred.unwrap_or_default(),
                            page.get_html_bytes_u8().len(),
                            match page.page_links {
                                Some(ref l) => l.len(),
                                _ => 0,
                            },
                            page.get_request().as_ref().map(|f| f.len()),
                            page.status_code
                        )
                        .as_bytes(),
                    )
                    .await;

                g.inc();
            }
        }
    );

    let duration = start.elapsed();

    println!(
        "Time elapsed in website.crawl({}) is: {:?} for total pages: {:?}",
        url,
        duration,
        links.len()
    );
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let _ = create_dir_all("./storage");

    let _ = tokio::join!(
        crawl_website("https://demo.fingerprint.com/playground"),
        crawl_website("https://bot-detector.rebrowser.net"),
        crawl_website("https://www.browserscan.net/bot-detection"),
        crawl_website("https://deviceandbrowserinfo.com/info_device"),
        crawl_website("https://deviceandbrowserinfo.com/are_you_a_bot"),
        crawl_website("https://abrahamjuliot.github.io/creepjs/"),
    );
}
