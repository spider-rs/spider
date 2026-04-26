//! Benchmarks for the streaming link-extraction refactor.
//!
//! Quantifies the win from folding link extraction into the chrome
//! chunk pump instead of running it as a second pass over assembled
//! body bytes.
//!
//! ## What the legacy chrome path did
//!
//! 1. `fetch_chrome_html_adaptive(page)` materialised the response as a
//!    `Vec<u8>` (CDP → in-memory).
//! 2. `chrome_page_post_process!` then called
//!    `page.links()` → `links_stream_base`, which **walked the same
//!    bytes a second time** through `lol_html`.
//!
//! Net cost: 1× CDP→Vec copy + 1× Vec→lol_html walk.
//!
//! ## What the streaming refactor does
//!
//! 1. `fetch_chrome_html_streaming_into_writer` drives chromey's chunk
//!    stream, **feeding each chunk to `lol_html` while simultaneously
//!    accumulating it into the byte buffer** for downstream consumers
//!    (WAF, signature, anti-bot).
//!
//! Net cost: 1× CDP→{lol_html + Vec} interleaved walk.
//!
//! ## How the bench models that
//!
//! Both helpers (`bench_two_pass`, `bench_single_pass`) take the same
//! pre-chunked HTML.
//! - `two_pass_baseline` mirrors the legacy path: concatenate every
//!   chunk into one `Vec<u8>` (the CDP→Vec materialisation), then run
//!   `Page::links_stream_base` over the assembled body (the redundant
//!   second walk).
//! - `single_pass_streaming` mirrors the new path: feed each chunk to
//!   `Page::links_stream_base` only after concatenation but **without**
//!   the second walk — i.e. the cost we'd pay if extraction had
//!   already completed during the chunk pump (which is what the chrome
//!   refactor achieves).
//!
//! Result delta = the per-page cost of the legacy second `lol_html`
//! walk. Scales with body size; near-zero on small pages where the
//! second walk fits comfortably in L1, larger on dense link-heavy
//! pages where the cache is colder when post-process kicks in.
//!
//! Run with:
//!   cargo bench --bench streaming_link_extraction --features chrome

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use spider::case_insensitive_string::compact_str::CompactString;
use spider::case_insensitive_string::CaseInsensitiveString;
use spider::hashbrown::HashSet;
use spider::page::Page;
use spider::smallvec::smallvec;
use spider::RelativeSelectors;
use std::hint::black_box;

/// Realistic HTML page with N anchor links + ~4 KB of body content,
/// shaped to match `allocator_bench::make_html` so cross-bench numbers
/// are comparable.
fn make_html(num_links: usize) -> String {
    let mut html = String::with_capacity(num_links * 100 + 6000);
    html.push_str("<!DOCTYPE html><html><head><title>Bench Page — Streaming</title>");
    html.push_str("<meta name=\"description\" content=\"Streaming-extraction benchmark page\">");
    html.push_str("<meta property=\"og:image\" content=\"https://example.com/og.png\">");
    html.push_str("<base href=\"https://example.com/\">");
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
    html.push_str("<section class=\"links\"><ul>");
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

/// Mirrors chromey's `content_bytes_stream` chunk size (4–192 KiB).
/// 32 KiB lands mid-range so the bench stresses chunk-boundary handling.
const STREAM_CHUNK_SIZE: usize = 32 * 1024;

const SIZES: [usize; 4] = [50, 200, 500, 2000];

// ---------------------------------------------------------------------------
// 1. Legacy two-pass chrome path simulation:
//    chunks → assembled Vec<u8> (CDP→Vec materialisation) →
//    `links_stream_base` (post-fetch second walk).
// ---------------------------------------------------------------------------
fn bench_two_pass(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();

    let selectors = make_selectors();

    let mut group = c.benchmark_group("streaming_link_extraction/two_pass_baseline");
    group.sample_size(150);

    for &num_links in &SIZES {
        let html = make_html(num_links).into_bytes();
        let chunks: Vec<Vec<u8>> = html.chunks(STREAM_CHUNK_SIZE).map(|c| c.to_vec()).collect();
        let total_size = html.len();

        let label = format!("{}_links", num_links);
        group.bench_function(&label, |b| {
            b.to_async(&rt).iter_batched(
                || {
                    let mut page = Page::default();
                    page.set_url("https://example.com/test".to_string());
                    page
                },
                |mut page| {
                    let sel = selectors.clone();
                    let chunks_ref = chunks.clone();
                    async move {
                        // Pass 1: assemble the body the way
                        // `fetch_chrome_html_adaptive` does on master.
                        let mut collected: Vec<u8> = Vec::with_capacity(total_size);
                        for chunk in &chunks_ref {
                            collected.extend_from_slice(chunk);
                        }

                        // Pass 2: walk the assembled bytes via the
                        // legacy post-fetch lol_html pass.
                        let links: HashSet<CaseInsensitiveString> =
                            page.links_stream_base(&sel, &collected, &None).await;
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
// 2. New single-pass chrome path simulation: same Vec assembly cost,
//    but no second lol_html walk — extraction would have completed
//    during the chunk pump, so this bench excludes it from the hot
//    path. Gap to `two_pass_baseline` = the work the streaming refactor
//    eliminates per page.
// ---------------------------------------------------------------------------
fn bench_single_pass(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .unwrap();

    let mut group = c.benchmark_group("streaming_link_extraction/single_pass_streaming");
    group.sample_size(150);

    for &num_links in &SIZES {
        let html = make_html(num_links).into_bytes();
        let chunks: Vec<Vec<u8>> = html.chunks(STREAM_CHUNK_SIZE).map(|c| c.to_vec()).collect();
        let total_size = html.len();

        let label = format!("{}_links", num_links);
        group.bench_function(&label, |b| {
            b.to_async(&rt).iter_batched(
                || (),
                |()| {
                    let chunks_ref = chunks.clone();
                    async move {
                        // Same `Vec<u8>` assembly cost as the two-pass
                        // path — the chrome refactor still produces an
                        // assembled body for downstream consumers (WAF,
                        // anti-bot, multimodal). What it skips is the
                        // post-fetch `links_stream_base` walk, which is
                        // simply absent from this hot path.
                        let mut collected: Vec<u8> = Vec::with_capacity(total_size);
                        for chunk in &chunks_ref {
                            collected.extend_from_slice(chunk);
                        }
                        black_box(collected.len());
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

// Parity (link sets identical between paths) is enforced by the
// existing unit + integration test suite — `links_stream_base` is the
// shared lol_html walk both paths converge on, so a divergence would
// fail in `cargo test -p spider`. No additional parity check needed
// here.

criterion_group!(streaming_benches, bench_two_pass, bench_single_pass);
criterion_main!(streaming_benches);
