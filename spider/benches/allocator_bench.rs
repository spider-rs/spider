//! Allocator stress benchmark — isolates allocation-heavy page processing
//! from network I/O to measure allocator impact.
//!
//! Run with:
//!   cargo bench --bench allocator_bench --features "default"
//!   cargo bench --bench allocator_bench --features "default,bench_jemalloc"
//!   cargo bench --bench allocator_bench --features "default,bench_mimalloc"

#[cfg(feature = "bench_jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "bench_mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use spider::case_insensitive_string::compact_str::CompactString;
use spider::case_insensitive_string::CaseInsensitiveString;
use spider::hashbrown::HashSet;
use spider::page::Page;
use spider::smallvec::smallvec;
use spider::RelativeSelectors;
use std::sync::Arc;

/// Generate a realistic HTML page with N links and ~4KB of body content
fn make_html(num_links: usize) -> String {
    let mut html = String::with_capacity(num_links * 100 + 6000);
    html.push_str("<!DOCTYPE html><html><head><title>Bench Page — Performance Test</title>");
    html.push_str(
        "<meta name=\"description\" content=\"A benchmark page for allocator stress testing\">",
    );
    html.push_str("<link rel=\"stylesheet\" href=\"/style.css\">");
    html.push_str("<script src=\"/app.js\"></script>");
    html.push_str("</head><body>");
    html.push_str("<header><nav><a href=\"/\">Home</a><a href=\"/about\">About</a></nav></header>");
    html.push_str("<main><div class=\"content\">");
    for i in 0..20 {
        html.push_str(&format!(
            "<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
             tempor incididunt ut labore et dolore magna aliqua. Paragraph {} of body content \
             that simulates realistic HTML density with enough text per element.</p>",
            i
        ));
    }
    html.push_str("</div><section class=\"links\"><ul>");
    for i in 0..num_links {
        html.push_str(&format!(
            "<li><a href=\"https://example.com/page/{}/detail?ref=nav&id={}\">Link {} — Category</a></li>\n",
            i, i * 7, i
        ));
    }
    html.push_str("</ul></section></main>");
    html.push_str("<footer><a href=\"/privacy\">Privacy</a><a href=\"/terms\">Terms</a></footer>");
    html.push_str("</body></html>");
    html
}

fn make_selectors() -> RelativeSelectors {
    (
        CompactString::from("example.com"),
        smallvec![
            CompactString::from("example.com"),
            CompactString::from("https"),
        ],
        CompactString::from("example.com"),
    )
}

// ---------------------------------------------------------------------------
// 1. Link extraction at various page sizes
// ---------------------------------------------------------------------------
fn bench_link_extraction(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();

    let html_50 = make_html(50);
    let html_200 = make_html(200);
    let html_500 = make_html(500);
    let selectors = make_selectors();
    let base = None;

    let mut group = c.benchmark_group("link_extraction");
    group.sample_size(200);

    for (label, html) in [
        ("50_links", &html_50),
        ("200_links", &html_200),
        ("500_links", &html_500),
    ] {
        let html = html.clone();
        group.bench_function(label, |b| {
            b.to_async(&rt).iter_batched(
                || {
                    let mut page = Page::default();
                    page.set_url("https://example.com/test".to_string());
                    page
                },
                |mut page| {
                    let sel = selectors.clone();
                    let b = base.clone();
                    let h = html.clone();
                    async move {
                        let links: HashSet<CaseInsensitiveString> =
                            page.links_stream_base(&sel, h.as_bytes(), &b).await;
                        black_box(links.len());
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// 2. HashSet allocation churn — single-threaded
// ---------------------------------------------------------------------------
fn bench_hashset_churn(c: &mut Criterion) {
    let mut group = c.benchmark_group("hashset_churn");
    group.sample_size(100);

    group.bench_function("2000_sets_x_80_urls", |b| {
        b.iter(|| {
            for _ in 0..2000 {
                let mut set: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
                for j in 0..80 {
                    set.insert(CaseInsensitiveString::from(format!(
                        "https://example.com/category/{}/page/{}?ref=crawl",
                        j % 10,
                        j
                    )));
                }
                black_box(set.len());
                // set dropped here — allocator must reclaim
            }
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 3. Concurrent allocation stress — simulates crawl spawn pattern
// ---------------------------------------------------------------------------
fn bench_concurrent_stress(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_stress");
    group.sample_size(80);

    // 8 threads, 500 "pages" each with 60 link allocations — simulates real crawl
    group.bench_function("8t_x_500_pages_x_60_links", |b| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(8)
            .enable_all()
            .build()
            .unwrap();

        b.to_async(&rt).iter(|| async {
            let mut handles = Vec::with_capacity(8);
            for t in 0..8u32 {
                handles.push(tokio::spawn(async move {
                    let mut total = 0usize;
                    for p in 0..500u32 {
                        // Simulate per-page link set creation
                        let mut links: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
                        for j in 0..60u32 {
                            links.insert(CaseInsensitiveString::from(format!(
                                "https://example.com/t{}/p{}/link/{}",
                                t, p, j
                            )));
                        }
                        total += links.len();

                        // Simulate URL string building (referer, target, etc.)
                        let _url = format!("https://example.com/t{}/p{}", t, p);
                        let _referer = format!("https://example.com/t{}/p{}", t, p.wrapping_sub(1));

                        // Simulate small vec of extracted metadata
                        let mut meta: Vec<String> = Vec::with_capacity(4);
                        meta.push(format!("title-{}-{}", t, p));
                        meta.push(format!("desc-{}-{}", t, p));
                        black_box(&meta);
                    }
                    black_box(total);
                }));
            }
            for h in handles {
                h.await.unwrap();
            }
        });
    });

    // 16 threads, smaller batches — tests allocator contention at high thread count
    group.bench_function("16t_x_200_pages_x_40_links", |b| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(16)
            .enable_all()
            .build()
            .unwrap();

        b.to_async(&rt).iter(|| async {
            let mut handles = Vec::with_capacity(16);
            for t in 0..16u32 {
                handles.push(tokio::spawn(async move {
                    for p in 0..200u32 {
                        let mut links: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
                        for j in 0..40u32 {
                            links.insert(CaseInsensitiveString::from(format!(
                                "https://cdn{}.example.com/asset/{}/{}",
                                t % 4,
                                p,
                                j
                            )));
                        }
                        black_box(links.len());
                    }
                }));
            }
            for h in handles {
                h.await.unwrap();
            }
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// 4. Mixed workload — interleaved small/large allocations like real crawl
// ---------------------------------------------------------------------------
fn bench_mixed_workload(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();

    let html_small = Arc::new(make_html(30));
    let html_medium = Arc::new(make_html(150));
    let html_large = Arc::new(make_html(400));
    let selectors = Arc::new(make_selectors());

    let mut group = c.benchmark_group("mixed_workload");
    group.sample_size(60);

    // Simulates a real crawl: mix of small, medium, large pages processed concurrently
    group.bench_function("8t_mixed_pages_100_each", |b| {
        b.to_async(&rt).iter(|| {
            let hs = html_small.clone();
            let hm = html_medium.clone();
            let hl = html_large.clone();
            let sel = selectors.clone();
            async move {
                let mut handles = Vec::with_capacity(8);
                for t in 0..8u32 {
                    let hs = hs.clone();
                    let hm = hm.clone();
                    let hl = hl.clone();
                    let sel = sel.clone();
                    handles.push(tokio::spawn(async move {
                        let base = None;
                        let mut total = 0usize;
                        for i in 0..100u32 {
                            let mut page = Page::default();
                            page.set_url(format!("https://example.com/t{}/p{}", t, i));
                            let html = match i % 3 {
                                0 => &*hs,
                                1 => &*hm,
                                _ => &*hl,
                            };
                            let links: HashSet<CaseInsensitiveString> =
                                page.links_stream_base(&sel, html.as_bytes(), &base).await;
                            total += links.len();
                        }
                        black_box(total);
                    }));
                }
                for h in handles {
                    h.await.unwrap();
                }
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_link_extraction,
    bench_hashset_churn,
    bench_concurrent_stress,
    bench_mixed_workload
);
criterion_main!(benches);
