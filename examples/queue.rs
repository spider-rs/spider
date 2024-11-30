//! `cargo run --example queue`
extern crate spider;

use spider::tokio;
use spider::url::Url;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr/en");
    let mut rx2 = website.subscribe(16).unwrap();
    let mut g = website.subscribe_guard().unwrap();
    let q = website.queue(100).unwrap();

    tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            let u = res.get_url();
            println!("{:?}", u);
            let mut url = Url::parse(u).expect("Failed to parse URL");

            let mut segments: Vec<_> = url
                .path_segments()
                .map(|c| c.collect::<Vec<_>>())
                .unwrap_or_else(Vec::new);

            if segments.len() > 0 && segments[0] == "en" {
                segments[0] = "fr";
                let new_path = segments.join("/");
                url.set_path(&new_path);
                // get a new url here or perform an action and queue links
                // pre-fetch all fr locales
                let _ = q.send(url.into());
            }

            g.inc();
        }
    });

    let start = std::time::Instant::now();
    website.crawl().await;
    let duration = start.elapsed();

    println!(
        "Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
        duration,
        website.get_links().len()
    )
}
