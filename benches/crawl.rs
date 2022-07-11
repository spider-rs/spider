use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::process::Command;
pub mod go_crolly;
pub mod node_crawler;

/// bench spider crawling between different libs
pub fn bench_speed(c: &mut Criterion) {
    let mut group = c.benchmark_group("crawl-speed/libraries");
    let sample_count = 10;
    let query = "https://rsseau.fr";
    let sample_title = format!("crawl {} samples", sample_count);

    group.sample_size(10);
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

/// bench spider crawling between different libs parallel 3x
pub fn bench_speed_parallel_x3(c: &mut Criterion) {
    let mut group = c.benchmark_group("crawl-speed-parallel/libraries");
    let sample_count = 10;
    let query = "https://rsseau.fr";
    let sample_title = format!("crawl {} samples", sample_count);

    group.sample_size(10);
    group.bench_function(format!("Rust[spider]: {}", sample_title), |b| {
        b.iter(|| {
            let args = ["--delay", "0", "--domain", &query, "crawl"];

            black_box(
                Command::new("spider")
                    .args(args)
                    .output()
                    .expect("rust command failed to start"),
            );
            black_box(
                Command::new("spider")
                    .args(args)
                    .output()
                    .expect("rust command failed to start"),
            );
            black_box(
                Command::new("spider")
                    .args(args)
                    .output()
                    .expect("rust command failed to start"),
            );
        })
    });
    group.bench_function(format!("Go[crolly]: {}", sample_title), |b| {
        b.iter(|| {
            black_box(
                Command::new("./gospider")
                    .output()
                    .expect("go command failed to start"),
            );
            black_box(
                Command::new("./gospider")
                    .output()
                    .expect("go command failed to start"),
            );
            black_box(
                Command::new("./gospider")
                    .output()
                    .expect("go command failed to start"),
            );
        })
    });
    group.bench_function(format!("Node.js[crawler]: {}", sample_title), |b| {
        let arg = "./node-crawler.js";

        b.iter(|| {
            black_box(
                Command::new("node")
                    .arg(arg)
                    .output()
                    .expect("node command failed to start"),
            );
            black_box(
                Command::new("node")
                    .arg(arg)
                    .output()
                    .expect("node command failed to start"),
            );
            black_box(
                Command::new("node")
                    .arg(arg)
                    .output()
                    .expect("node command failed to start"),
            );
        })
    });
    group.bench_function(format!("C[wget]: {}", sample_title), |b| {
        b.iter(|| {
            let args = [
                "-4",
                "--recursive",
                "--no-parent",
                "--ignore-tags=img,link,script",
                "--spider",
                "-q",
                &query,
            ];

            black_box(
                Command::new("wget")
                    .args(args)
                    .output()
                    .expect("wget command failed to start"),
            );

            black_box(
                Command::new("wget")
                    .args(args)
                    .output()
                    .expect("wget command failed to start"),
            );

            black_box(
                Command::new("wget")
                    .args(args)
                    .output()
                    .expect("wget command failed to start"),
            );
        })
    });
    group.finish();
}

criterion_group!(benches, bench_speed, bench_speed_parallel_x3);
criterion_main!(benches);
