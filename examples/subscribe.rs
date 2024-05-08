//! `cargo run --example subscribe`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    let mut rx2: tokio::sync::broadcast::Receiver<spider::page::Page> =
        website.subscribe(0).unwrap();

    tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();

        while let Ok(res) = rx2.recv().await {
            let _ = stdout
                .write_all(format!("- {}\n", res.get_url()).as_bytes())
                .await;
        }
    });

    let start = std::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        website.get_links().len()
    )
}
