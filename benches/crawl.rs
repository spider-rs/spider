use criterion::{criterion_group, criterion_main, Criterion};
use spider::website::Website;

#[inline]
fn crawl() {
    let mut website: Website = Website::new("https://rsseau.fr");
    website.configuration.respect_robots_txt = true;
    website.crawl();
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("crawl-duration-example");

    group.significance_level(0.1).sample_size(10);
    group.bench_function("crawl 10 times", |b| b.iter(|| crawl()));
    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
