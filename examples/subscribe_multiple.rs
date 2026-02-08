//! `cargo run --example subscribe_multiple`
extern crate spider;

use spider::{tokio, website::Website};
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://example.com?target=1");

    let mut rx2: tokio::sync::broadcast::Receiver<spider::page::Page> =
        website.subscribe(0).unwrap();

    let mut website2 = website.clone();
    website2.set_url_only("https://example.com?target=2");
    // usually you want to use another proxy.
    // website2.with_proxies(Some(vec!["http://myproxy.com"]));

    let mut stdout = tokio::io::stdout();

    let sub = async move {
        while let Ok(res) = rx2.recv().await {
            let _ = stdout
                .write_all(format!("- {}\n", res.get_url()).as_bytes())
                .await;
        }
        stdout
    };

    let start = std::time::Instant::now();

    let c1 = async {
        website.crawl().await;
        website.unsubscribe();
    };
    let c2 = async {
        website2.crawl().await;
        website2.unsubscribe();
    };

    // you can also use a select to cancel a crawl if you want to see which proxy comes first.
    let (mut stdout, _crawl_one, _crawl_two) = tokio::join!(sub, c1, c2);

    let duration = start.elapsed();

    let _ = stdout
        .write_all(
            format!(
                "Time elapsed in website.crawl() and website2.crawl() is: {:?} for total pages: {:?}",
                duration,
                website.get_size().await +  website2.get_size().await
            )
            .as_bytes(),
        )
        .await;
}
