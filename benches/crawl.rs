pub mod go_crolly;
pub mod node_crawler;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use spider::website::Website;
use std::{env, process::Command};

/// bench crawling between different libs
pub fn bench_speed(c: &mut Criterion) {
    let mut worker: Option<std::process::Child> = None;
    let query = env::var("SPIDER_BENCH_URL").unwrap_or("https://spider.cloud".into());

    if cfg!(feature = "decentralized") {
        worker.replace(
            std::process::Command::new("spider_worker")
                .spawn()
                .expect("spider_worker command failed to start"),
        );
    }

    let mut group = c.benchmark_group("crawl-speed/libraries");
    let sample_count = 10;

    let sample_title = format!("crawl {} samples", sample_count);

    group.sample_size(sample_count);

    let rt = spider::tokio::runtime::Runtime::new().unwrap();

    group.bench_function(format!("Rust[spider]: {}", sample_title), |b| {
        let mut website = Website::new(&query);
        // try to only enable when you know you can or have permission from the website to avoid being blocked.
        website.configuration.respect_robots_txt = false;

        b.to_async(&rt)
            .iter(|| black_box(async_crawl_single(website.clone())))
    });

    drop(rt);
    match worker.take() {
        Some(mut worker) => worker.kill().expect("spider_worker wasn't running"),
        _ => (),
    };
    drop(worker);

    // [deprecated] - lib does not update and over 60x slower.
    if env::var("BENCH_GO_CROLLY").unwrap_or_default() == "true" {
        group.bench_function(format!("Go[crolly]: {}", sample_title), |b| {
            b.iter(|| {
                black_box(
                    Command::new("./gospider")
                        .arg(&query)
                        .output()
                        .expect("go command failed to start"),
                )
            })
        });
    }

    // nodejs runs the cpu fully to 100%. Small test can run fast - does not scale with concurrency.
    if env::var("BENCH_NODE_CRAWLER").unwrap_or_default() == "true" {
        group.bench_function(format!("Node.js[crawler]: {}", sample_title), |b| {
            b.iter(|| {
                black_box(
                    Command::new("node")
                        .arg("./node-crawler.js")
                        .arg(&query)
                        .output()
                        .expect("node command failed to start"),
                )
            })
        });
    }

    // network latency drawbacks - only compare locale examples.
    if env::var("BENCH_C_WGET").unwrap_or_default() == "true" {
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
    }

    group.finish();
}

/// crawl threaded internal test single
async fn async_crawl_single(mut website: Website) {
    website.crawl().await;
}

criterion_group!(benches, bench_speed);
criterion_main!(benches);
