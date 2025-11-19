//! `cargo run --example callback`
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr/en");

    let some = "custom";

    website.set_on_link_find(move |s, ss| {
        println!("link target: {:?} - {some}", s);
        // forward link to a different destination
        (s.as_ref().replacen("/fr/", "", 1).into(), ss)
    });

    website.crawl().await;
}
