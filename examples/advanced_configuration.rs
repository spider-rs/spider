//! `cargo run --example advanced_configuration`
extern crate spider;

use spider::configuration::Configuration;
use spider::{tokio, website::Website};
use std::io::Error;
use std::time::Instant;

const CAPACITY: usize = 5;
const CRAWL_LIST: [&str; CAPACITY] = [
    "https://rsseau.fr",
    "https://choosealicense.com",
    "https://jeffmendez.com",
    "https://spider-rs.github.io/spider-nodejs/",
    "https://spider-rs.github.io/spider-py/",
];

#[tokio::main]
async fn main() -> Result<(), Error> {
    let config = Configuration::new()
        .with_user_agent(Some("SpiderBot"))
        .with_blacklist_url(Some(Vec::from(["https://rsseau.fr/resume".into()])))
        .with_subdomains(false)
        .with_tld(false)
        .with_redirect_limit(3)
        .with_respect_robots_txt(true)
        .with_external_domains(Some(
            Vec::from(["http://loto.rsseau.fr/"].map(|d| d.to_string())).into_iter(),
        ))
        .build();

    let mut handles = Vec::with_capacity(CAPACITY);

    for website_url in CRAWL_LIST {
        match Website::new(website_url)
            .with_config(config.to_owned())
            .build()
        {
            Ok(mut website) => {
                let handle = tokio::spawn(async move {
                    println!("Starting Crawl - {:?}", website.get_domain().inner());

                    let start = Instant::now();
                    website.crawl().await;
                    let duration = start.elapsed();

                    let links = website.get_links();

                    for link in links {
                        println!("- {:?}", link.as_ref());
                    }

                    println!(
                        "{:?} - Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
                        website.get_domain().inner(),
                        duration,
                        links.len()
                    );
                });

                handles.push(handle);
            }
            Err(e) => println!("{:?}", e),
        }
    }

    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}
