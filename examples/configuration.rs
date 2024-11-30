//! `cargo run --example configuration`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::io::Error;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut website = Website::new("https://rsseau.fr/en")
        .with_user_agent(Some("SpiderBot"))
        .with_blacklist_url(Some(Vec::from(["https://rsseau.fr/resume".into()])))
        .with_subdomains(true)
        .with_concurrency_limit(None)
        .with_tld(true)
        .with_respect_robots_txt(true)
        .with_external_domains(Some(
            Vec::from(["http://loto.rsseau.fr/"].map(|d| d.to_string())).into_iter(),
        ))
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
