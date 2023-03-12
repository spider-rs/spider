//! `cargo run --example serde --features serde`
extern crate spider;
extern crate flexbuffers;

use spider::tokio;
use spider::website::Website;
use spider::serde::ser::Serialize;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr");

    website.crawl().await;

    let links = website.get_links();

    let mut s = flexbuffers::FlexbufferSerializer::new();

    links.serialize(&mut s).unwrap();

    println!(
        "{:?}",
        s
    )
}
