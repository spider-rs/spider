use criterion::{criterion_group, black_box, criterion_main, Criterion};
use std::process::{Command};
use std::time::Duration;
pub mod node_crawler;
pub mod go_crolly;

/// bench spider crawling between different libs
pub fn bench_speed(c: &mut Criterion) {
    let mut group = c.benchmark_group("crawl-speed/libraries");
    let node_crawl_script = node_crawler::gen_crawl();
    let go_crawl_script = go_crolly::gen_crawl();
        
    group.sample_size(10).measurement_time(Duration::new(180, 0) + Duration::from_millis(500));
    group.bench_function("Rust[spider]: with crawl 10 times", |b| b.iter(||black_box(Command::new("spider")
        .args(["--delay", "0", "--domain", "https://rsseau.fr", "crawl"])
        .output()
        .expect("rust command failed to start"))
    ));
    group.bench_function("Go[crolly]: with crawl 10 times", |b| b.iter(||black_box(Command::new("go")
        .arg("run")
        .arg(&go_crawl_script)
        .output()
        .expect("go command failed to start"))
    ));
    group.bench_function("Node.js[crawler]: with crawl 10 times", |b| b.iter(|| black_box(Command::new("node")
        .arg(&node_crawl_script)
        .output()
        .expect("node command failed to start"))
    ));
    group.finish();
}

criterion_group!(benches, bench_speed);
criterion_main!(benches);
