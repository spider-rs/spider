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
    let query = "http://localhost:3000";
    let sample_title = format!("crawl {} samples", sample_count);

    Command::new("npm")
        .args(["--prefix", "./portfolio", "run", "serve", "--", "-p", "3000"])
        .spawn()
        .expect("http local server command failed to start");

    group.sample_size(sample_count);
    group.bench_function(format!("Rust[spider]: {}", sample_title), |b| {
        let mut website: Website = Website::new(&query);
        website.configuration.delay = 0;

        b.iter(move || {
            black_box(website.crawl())
        })
    });
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
                        "--spider",
                        "-r",
                        &query,
                    ])
                    .output()
                    .expect("wget command failed to start"),
            )
        })
    });
    group.finish();

    Command::new("kill")
    .arg("$(lsof -t -i:3000)")
    .output()
    .expect("shutting down http server failed");
}

/// bench concurrent crawling between different libs parallel 10x
pub fn bench_speed_concurrent_x10(c: &mut Criterion) {
    let mut group = c.benchmark_group("crawl-speed-concurrent/libraries");
    let sample_count = 10;
    let query = "http://localhost:3000";
    let sample_title = format!("crawl concurrent {} samples", sample_count);
    let concurrency_count: Vec<_> = (0..10).collect();

    group.sample_size(sample_count);
    group.bench_function(format!("Rust[spider]: {}", sample_title), |b| {
        b.iter(|| {
            let threads: Vec<_> = concurrency_count
            .clone()
            .into_iter()
            .map(|_| {
                thread::spawn(move || {
                    let mut website: Website = Website::new(&query);
                    website.configuration.delay = 0;
                    black_box(website.crawl());
                })
                })
                .collect();

            for handle in threads {
                handle.join().unwrap()
            }
        })
    });

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
                                    "--spider",
                                    "-r",
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

criterion_group!(benches, bench_speed, bench_speed_concurrent_x10);
criterion_main!(benches);
