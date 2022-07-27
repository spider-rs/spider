pub mod go_crolly;
pub mod node_crawler;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::process::Command;
use std::thread;

/// bench crawling between different libs
pub fn bench_speed(c: &mut Criterion) {
    let mut group = c.benchmark_group("crawl-speed/libraries");
    let sample_count = 10;
    let query = "https://rsseau.fr";
    let sample_title = format!("crawl {} samples", sample_count);

    group.sample_size(sample_count);
    group.bench_function(format!("Rust[spider]: {}", sample_title), |b| {
        b.iter(|| {
            black_box(
                Command::new("spider")
                    .args(["--delay", "0", "--domain", &query, "crawl"])
                    .output()
                    .expect("rust command failed to start"),
            )
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

    group.sample_size(sample_count);
    group.bench_function(format!("Rust[spider]: {}", sample_title), |b| {
        b.iter(|| {
            let threads: Vec<_> = concurrency_count
                .clone()
                .into_iter()
                .map(|_| {
                    thread::spawn(move || {
                        black_box(
                            Command::new("spider")
                                .args(["--delay", "0", "--domain", &query, "crawl"])
                                .output()
                                .expect("rust command failed to start"),
                        );
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

criterion_group!(benches, bench_speed, bench_speed_concurrent_x10);
criterion_main!(benches);
