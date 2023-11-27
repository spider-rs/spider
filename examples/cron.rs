//! `cargo run --example cron`
extern crate spider;

use spider::tokio;
use spider::website::{Website, run_cron};

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");
    website.cron_str = "1/5 * * * * *".into();

    let mut rx2 = website.subscribe(16).unwrap();

   let join_handle = tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            println!("{:?}", res.get_url());
        }
    });

    let mut runner = run_cron(website).await;

    println!("Starting the Runner for 20 seconds");
    tokio::time::sleep(tokio::time::Duration::from_secs(20)).await;
    let _ = tokio::join!(runner.stop(), join_handle);
}
