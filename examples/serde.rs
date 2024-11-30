//! `cargo run --example serde --features serde`
extern crate flexbuffers;
extern crate spider;

use spider::serde::ser::Serialize;
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr/en");

    website.crawl().await;

    let links = website.get_links();

    let mut s = flexbuffers::FlexbufferSerializer::new();

    links.serialize(&mut s).unwrap();

    println!("{:?}", s)
}
