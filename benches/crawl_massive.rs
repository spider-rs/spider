//! `cargo bench --bench crawl_massive`
extern crate spider;

use spider::tokio;
use spider::website::Website;
use std::time::Instant;

#[tokio::main]
async fn main() {
    let mut worker: Option<std::process::Child> = None;

    if cfg!(feature = "decentralized") {
        worker.replace(
            std::process::Command::new("spider_worker")
                .spawn()
                .expect("spider_worker command failed to start"),
        );
    }

    let target = match std::env::var("SPIDER_BENCH_URL_LARGE") {
        Ok(v) => v,
        _ => "https://wikipedia.org".into()
    };

    let mut website: Website = Website::new(&target);
    website.configuration.user_agent = Some(Box::new("usasearch".into())); // Defaults to spider/x.y.z, where x.y.z is the library version
    website.configuration.respect_robots_txt = true;
    let start = Instant::now();
    website.crawl().await;
    let duration = start.elapsed();
    let links = website.get_links();
    match worker.take() {
        Some(mut worker) => worker.kill().expect("spider_worker wasn't running"),
        _ => (),
    };
    drop(worker);

    println!(
        "Time elapsed in website.crawl() is: {:?} for {} with total pages: {:?}",
        duration,
        target,
        links.len()
    )
}
