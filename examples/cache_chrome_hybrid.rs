//! cargo run --example cache_chrome_hybrid --features="spider/sync spider/chrome spider/cache_chrome_hybrid"
extern crate spider;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::spider::http_cache_reqwest::CacheManager;
use crate::spider::tokio::io::AsyncWriteExt;
use spider::string_concat::{string_concat, string_concat_impl};
use spider::tokio;
use spider::website::Website;

static GLOBAL_URL_COUNT: AtomicUsize = AtomicUsize::new(0);

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr")
        .with_caching(true)
        .build()
        .unwrap();
    let mut rx2 = website.subscribe(500).unwrap();

    let start = std::time::Instant::now();

    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();

        while let Ok(res) = rx2.recv().await {
            let message = format!("CACHING - {:?}\n", res.get_url());
            let _ = stdout.write_all(message.as_bytes()).await;
        }
    });

    website.crawl().await;
    website.unsubscribe();

    let mut rx2 = website.subscribe(500).unwrap();

    let subscription = async move {
        while let Ok(res) = rx2.recv().await {
            let mut stdout = tokio::io::stdout();
            let cache_url = string_concat!("GET:", res.get_url());

            tokio::task::spawn(async move {
                let result = tokio::time::timeout(Duration::from_millis(60), async {
                    spider::website::CACACHE_MANAGER.get(&cache_url).await
                })
                .await;

                match result {
                    Ok(Ok(Some(_cache))) => {
                        let message = format!("HIT - {:?}\n", cache_url);
                        let _ = stdout.write_all(message.as_bytes()).await;
                    }
                    Ok(Ok(None)) | Ok(Err(_)) => {
                        let message = format!("MISS - {:?}\n", cache_url);
                        let _ = stdout.write_all(message.as_bytes()).await;
                    }
                    Err(_) => {
                        let message = format!("ERROR - {:?}\n", cache_url);
                        let _ = stdout.write_all(message.as_bytes()).await;
                    }
                };

                GLOBAL_URL_COUNT.fetch_add(1, Ordering::Relaxed);
            });
        }
    };

    let crawl = async move {
        website.crawl_raw().await;
        website.unsubscribe();
    };

    tokio::pin!(subscription);

    tokio::select! {
        _ = crawl => (),
        _ = subscription => (),
    };

    let duration = start.elapsed();

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration, GLOBAL_URL_COUNT
    )
}
