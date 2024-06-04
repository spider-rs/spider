//! `cargo run --example depth`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::io::Error;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut website = Website::new("https://rsseau.fr")
        .with_depth(3)
        .build()
        .unwrap();

    let start = Instant::now();
    website.crawl().await;
    let duration: std::time::Duration = start.elapsed();

    let links = website.get_links();

    for link in links {
        println!("- {:?}", link.as_ref());
    }

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        links.len()
    );

    Ok(())
}
