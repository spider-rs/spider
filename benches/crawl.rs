pub mod go_crolly;
pub mod node_crawler;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use spider::website::Website;
use std::process::Command;
use std::thread;

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
            .iter(|| black_box(async_crawl_single(&website)))
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

/// bench concurrent crawling between different libs parallel 10x
pub fn bench_speed_concurrent_x10(c: &mut Criterion) {
    let mut group = c.benchmark_group("crawl-speed-concurrent/libraries");
    let sample_count = 10;
    let query = "https://rsseau.fr";
    let sample_title = format!("crawl concurrent {} samples", sample_count);
    let concurrency_count: Vec<_> = (0..10).collect();

    let rt = spider::tokio::runtime::Runtime::new().unwrap();

    group.sample_size(sample_count);
    group.bench_function(format!("Rust[spider]: {}", sample_title), |b| {
        let mut website = Website::new(&query);
        website.configuration.delay = 0;
        website.configuration.respect_robots_txt = false;

        let c = concurrency_count.clone();
        b.to_async(&rt)
            .iter(|| black_box(async_crawl_multi(&website, &c)))
    });

    drop(rt);

    group.bench_function(format!("Go[crolly]: {}", sample_title), |b| {
        b.iter(|| {
            let threads: Vec<_> = concurrency_count
                .clone()
                .into_iter()
                .map(|_| {
                    thread::spawn(move || {
                        black_box(
                            Command::new("./gospider")
                                .output()
                                .expect("go command failed to start"),
                        );
                    })
                })
                .collect();

            for handle in threads {
                handle.join().unwrap()
            }
        })
    });

    group.bench_function(format!("Node.js[crawler]: {}", sample_title), |b| {
        b.iter(|| {
            let threads: Vec<_> = concurrency_count
                .clone()
                .into_iter()
                .map(|_| {
                    thread::spawn(move || {
                        black_box(
                            Command::new("node")
                                .arg("./node-crawler.js")
                                .output()
                                .expect("node command failed to start"),
                        );
                    })
                })
                .collect();

            for handle in threads {
                handle.join().unwrap()
            }
        })
    });

    group.bench_function(format!("C[wget]: {}", sample_title), |b| {
        b.iter(|| {
            let threads: Vec<_> = concurrency_count
                .clone()
                .into_iter()
                .map(|_| {
                    thread::spawn(move || {
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
                        );
                    })
                })
                .collect();

            for handle in threads {
                handle.join().unwrap()
            }
        })
    });
    group.finish();
}

/// crawl threaded internal test
async fn async_crawl_multi(website: &Website, concurrency_count: &Vec<u32>) {
    let threads: Vec<_> = concurrency_count
        .clone()
        .into_iter()
        .map(|_| {
            let mut website = website.clone();
            spider::tokio::task::spawn(async move {
                website.crawl().await;
            })
        })
        .collect();

    for handle in threads {
        handle.await.unwrap();
    }
}

/// crawl threaded internal test single
async fn async_crawl_single(website: &Website) {
    let mut website = website.clone();
    website.crawl().await;
}

criterion_group!(benches, bench_speed, bench_speed_concurrent_x10);
criterion_main!(benches);
