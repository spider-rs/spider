//! cargo run --example url_glob --features glob
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new(
        "https://rsseau.fr/blog/{lazy-load-components,gnu-parallel,migrate-from-jekyll-to-gatsby}",
    );
    website
        .configuration
        .blacklist_url
        .insert(Default::default())
        .push("https://rsseau.fr/resume".into());

    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    let links = website.get_links();

    for link in links.iter() {
        println!("- {:?}", link.as_ref());
    }

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    )
}
