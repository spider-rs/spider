//! cargo run --example concurrent_profiles --features="chrome chrome_intercept disk"

extern crate spider;
use crate::spider::tokio::io::AsyncWriteExt;
use spider::configuration::Fingerprint;
use spider::features::chrome_common::RequestInterceptConfiguration;

use spider::tokio;
use spider::utils::split_hashset_round_robin;
use spider::website::Website;
use std::io::Result;

const CRAWL_LIMIT: u32 = 50;

async fn crawl_website(url: &str) -> Result<()> {
    let mut website: Website = Website::new(url)
        .with_limit(CRAWL_LIMIT)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true)
        .with_shared_state(true)
        .with_sqlite(true)
        .with_fingerprint(true)
        .with_proxies(Some(vec!["http://localhost:9111".into()]))
        .with_chrome_connection(Some("http://127.0.0.1:9222/json/version".into()))
        .build()
        .unwrap();

    let shared_db = website.setup_database_handler();

    let pool = shared_db.generate_pool().await;

    shared_db.set_pool(pool).await;
    website.setup_shared_db(shared_db);

    website.set_disk_persistance(true);

    let mut rx2 = website.subscribe(16).unwrap();
    let mut website1 = website.clone();

    let website1 = website1
        .with_proxies(Some(vec!["http://localhost:9112".into()]))
        .with_fingerprint_advanced(Fingerprint::Basic);

    let mut stdout = tokio::io::stdout();

    tokio::spawn(async move {
        let mut total_links = 0;
        while let Ok(page) = rx2.recv().await {
            let _ = stdout
                .write_all(
                    format!(
                        "- {} -- Bytes transferred {:?} -- HTML Size {:?}\n",
                        page.get_url(),
                        page.bytes_transferred.unwrap_or_default(),
                        page.get_html_bytes_u8().len()
                    )
                    .as_bytes(),
                )
                .await;
            total_links += 1;
        }
        println!("Total links: {:?}", total_links);
    });

    let start = crate::tokio::time::Instant::now();

    website.crawl_raw().await;

    website.persist_links();
    website1.persist_links();

    let extra_links = website.drain_extra_links();
    let extra_links: spider::hashbrown::HashSet<spider::CaseInsensitiveString> =
        extra_links.collect();

    let mut set: Vec<spider::hashbrown::HashSet<spider::CaseInsensitiveString>> =
        split_hashset_round_robin(extra_links, 2);

    website1.set_extra_links(set.remove(0));
    website.set_extra_links(set.remove(0));

    website1.with_limit(CRAWL_LIMIT * 2);
    website.with_limit(CRAWL_LIMIT * 2);

    website.set_disk_persistance(false);
    website1.set_disk_persistance(false);

    println!("STARTING CONCURRENT RUN HTTP-CHROME");

    tokio::join!(website.crawl_raw(), website1.crawl());

    let duration = start.elapsed();

    let links = website.get_all_links_visited().await;

    println!(
        "Time elapsed in website.crawl({}) is: {:?} for total pages: {:?}",
        website.get_url(),
        duration,
        links.len()
    );

    website.clear_all().await;
    website1.clear_all().await;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let _ = tokio::join!(
        crawl_website("https://choosealicense.com"),
        crawl_website("https://spider.cloud"),
    );

    Ok(())
}
