//! `cargo run --example transform_markdown --features="spider/sync spider_utils/transformations"`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use spider_utils::spider_transformations::transformation::content::{
    transform_content, ReturnFormat, TransformConfig,
};
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr/en");
    let mut rx2: tokio::sync::broadcast::Receiver<spider::page::Page> =
        website.subscribe(0).unwrap();
    let mut stdout = tokio::io::stdout();

    let mut conf = TransformConfig::default();
    conf.return_format = ReturnFormat::Markdown;

    let join_handle = tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            let markup = transform_content(&res, &conf, &None, &None, &None);

            let _ = stdout
                .write_all(format!("- {}\n {}\n", res.get_url(), markup).as_bytes())
                .await;
        }
        stdout
    });

    let start = std::time::Instant::now();
    website.crawl().await;
    website.unsubscribe();
    let duration = start.elapsed();
    let mut stdout = join_handle.await.unwrap();

    let _ = stdout
        .write_all(
            format!(
                "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
                duration,
                website.get_size().await
            )
            .as_bytes(),
        )
        .await;
}
