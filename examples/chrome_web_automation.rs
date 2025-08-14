//! `cargo run --example chrome_web_automation --features chrome chrome_headed`
extern crate spider;

use std::time::Duration;

use spider::configuration::{WaitForIdleNetwork, WebAutomation};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::HashMap;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut automation_scripts = HashMap::new();

    automation_scripts.insert(
        "/en/blog".into(),
        Vec::from([
            WebAutomation::Evaluate(r#"document.body.style.background = "blue";"#.into()),
            WebAutomation::ScrollY(2000),
            WebAutomation::Click("article a".into()),
            WebAutomation::Wait(5000),
            WebAutomation::Screenshot {
                output: "example.png".into(),
                full_page: true,
                omit_background: true,
            },
        ]),
    );
    let mut tracker = spider::configuration::ChromeEventTracker::new(true, true);
    tracker.automation = true;

    let mut website: Website = Website::new("https://rsseau.fr/en/blog")
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(30)))))
        .with_limit(1)
        .with_event_tracker(Some(tracker))
        .with_automation_scripts(Some(automation_scripts))
        .build()
        .unwrap();

    let mut rx2 = website.subscribe(16).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx2.recv().await {
            println!("{:?} - {:?}", page.get_url(), page.get_metadata());
        }
    });

    let start = crate::tokio::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;

    for link in links.iter() {
        println!("- {:?}", link.as_ref());
    }

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}
