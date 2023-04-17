pub mod go_crolly;
pub mod node_crawler;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use spider::website::Website;
use std::process::Command;

/// bench crawling between different libs
pub fn bench_speed(c: &mut Criterion) {
    let mut group = c.benchmark_group("crawl-speed/libraries");
    let sample_count = 10;
    let query = "https://rsseau.fr";
    let sample_title = format!("crawl {} samples", sample_count);

    group.sample_size(sample_count);

    let rt = spider::tokio::runtime::Runtime::new().unwrap();

    group.bench_function(format!("Rust[spider]: {}", sample_title), |b| {
        let mut website = Website::new(&query);
        website.configuration.delay = 0;
        website.configuration.respect_robots_txt = false;

        b.to_async(&rt)
            .iter(|| black_box(async_crawl_single(website.clone())))
    });

    drop(rt);

    group.bench_function(format!("Go[crolly]: {}", sample_title), |b| {
        b.iter(|| {
            black_box(
                Command::new("./gospider")
                    .output()
                    .expect("go command failed to start"),
            )
        })
    });
    group.bench_function(format!("Node.js[crawler]: {}", sample_title), |b| {
        b.iter(|| {
            black_box(
                Command::new("node")
                    .arg("./node-crawler.js")
                    .output()
                    .expect("node command failed to start"),
            )
        })
    });
    group.bench_function(format!("C[wget]: {}", sample_title), |b| {
        b.iter(|| {
            black_box(
                Command::new("wget")
                    .args([
                        "-4",
                        "--recursive",
                        "--no-parent",
                        "--ignore-tags=img,link,script",
                        "--spider",
                        "-q",
                        &query,
                    ])
                    .output()
                    .expect("wget command failed to start"),
            )
        })
    });
    group.finish();
}

/// crawl threaded internal test single
async fn async_crawl_single(mut website: Website) {
    website.crawl().await;
}

criterion_group!(benches, bench_speed);
criterion_main!(benches);
