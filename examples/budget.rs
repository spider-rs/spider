//! `cargo run --example budget --features budget`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::io::Error;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut website = Website::new("https://rsseau.fr/en")
        .with_budget(Some(spider::hashbrown::HashMap::from([
            ("*", 15),
            ("en", 11),
            ("fr", 3),
        ])))
        .with_limit(15) // this does the same as the above "*", 15. Setting a limit to the amount of pages gathered.
        .build()
        .unwrap();

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
    );

    Ok(())
}
