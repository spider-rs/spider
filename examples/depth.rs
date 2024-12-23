//! `cargo run --example depth`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::io::Error;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut website = Website::new("https://rsseau.fr/en")
        .with_depth(3)
        .build()
        .unwrap();

    let start = Instant::now();
    website.crawl().await;
    let duration: std::time::Duration = start.elapsed();

    let links = website.get_all_links_visited().await;

    for link in links.iter() {
        println!("- {:?}", link.as_ref());
    }

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    );

    Ok(())
}
