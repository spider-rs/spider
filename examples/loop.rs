//! `cargo run --example loop --features="spider/sync spider/smart"`
extern crate spider;

use spider::website::Website;
use spider::{configuration, tokio};
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() {
    let mut configuration = configuration::Configuration::new();

    configuration.with_limit(250);

    let website_list = vec![
        "https://rsseau.fr",
        "https://choosealicense.com",
        "https://a11ywatch.com",
        "https://spider.cloud",
    ];

    let mut tasks = Vec::new();

    for u in website_list {
        let configuration = configuration.clone();

        tasks.push(tokio::spawn(async move {
            let mut website: Website = Website::new(u).with_config(configuration).build().unwrap();
            let mut rx2: tokio::sync::broadcast::Receiver<spider::page::Page> =
                website.subscribe(0).unwrap();

            let mut stdout = tokio::io::stdout();

            let join_handle = tokio::spawn(async move {
                while let Ok(res) = rx2.recv().await {
                    let _ = stdout
                        .write_all(format!("- {}\n", res.get_url()).as_bytes())
                        .await;
                }
                stdout
            });

            let start = std::time::Instant::now();
            if u == "https://spider.cloud" {
                website.crawl().await;
            } else {
                website.crawl_raw().await;
            };

            website.unsubscribe();
            let duration = start.elapsed();
            let mut stdout = join_handle.await.unwrap();

            let _ = stdout
                .write_all(
                    format!(
                        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}\n",
                        duration,
                        website.get_links().len()
                    )
                    .as_bytes(),
                )
                .await;
        }));
    }

    let start = std::time::Instant::now();

    for task in tasks {
        let _ = task.await;
    }

    let duration = start.elapsed();
    let mut stdout = tokio::io::stdout();

    let _ = stdout
        .write_all(
            format!(
                "Total time elapsed for all website.crawl() is: {:?}\n",
                duration,
            )
            .as_bytes(),
        )
        .await;
}
