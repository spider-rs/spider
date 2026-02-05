use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use spider::hashbrown::{HashMap, HashSet};
use spider::packages::robotparser::parser::RobotFileParser;
use spider::utils::abs::{convert_abs_path, parse_absolute_url};
use spider::utils::interner::ListBucket;
use spider::utils::trie::Trie;
use spider::utils::{
    clean_html_base, clean_html_full, clean_html_slim, detect_antibot_from_url, flip_http_https,
    get_last_segment, prepare_url,
};
use spider::CaseInsensitiveString;

// ---------------------------------------------------------------------------
// Fixture generators
// ---------------------------------------------------------------------------

/// Build a realistic HTML string with various elements.
fn generate_html(num_elements: usize) -> String {
    let mut html = String::from(
        "<!DOCTYPE html><html><head><title>Bench</title>\
         <meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\">\
         <style>body{margin:0}nav{display:flex}.hero{padding:2rem}</style>\
         <script>window.__data={init:true};</script></head><body>\
         <nav><a href=\"/\">Home</a><a href=\"/about\">About</a></nav>\
         <main>",
    );
    for i in 0..num_elements {
        match i % 7 {
            0 => html.push_str(&format!(
                "<div class=\"card\" id=\"c{i}\"><h2>Title {i}</h2><p>Description for item {i} with some text.</p></div>"
            )),
            1 => html.push_str(&format!(
                "<a href=\"/page/{i}\" class=\"link\">Link {i}</a>"
            )),
            2 => html.push_str(&format!(
                "<script>console.log('script-{i}');</script>"
            )),
            3 => html.push_str(&format!(
                "<style>.item-{i}{{color:red;font-size:14px}}</style>"
            )),
            4 => html.push_str(&format!(
                "<svg viewBox=\"0 0 100 100\"><circle cx=\"50\" cy=\"50\" r=\"{}\"/></svg>",
                i % 50
            )),
            5 => html.push_str(&format!(
                "<img src=\"/img/{i}.jpg\" alt=\"Image {i}\" loading=\"lazy\">"
            )),
            _ => html.push_str(&format!(
                "<p data-id=\"{i}\">Paragraph {i} with <strong>bold</strong> and <em>italic</em> text.</p>"
            )),
        }
    }
    html.push_str("</main><footer><p>Copyright 2025</p></footer></body></html>");
    html
}

/// Build a multi-agent robots.txt with Allow/Disallow rules.
fn generate_robots_txt(num_entries: usize) -> Vec<String> {
    let mut lines = Vec::with_capacity(num_entries + 10);
    lines.push("User-agent: *".to_string());
    lines.push("Crawl-delay: 1".to_string());
    for i in 0..num_entries {
        if i % 3 == 0 {
            lines.push(format!("Disallow: /private/{i}/"));
        } else {
            lines.push(format!("Allow: /public/{i}/"));
        }
    }
    lines.push(String::new());
    lines.push("User-agent: Googlebot".to_string());
    lines.push("Allow: /".to_string());
    lines.push(String::new());
    lines.push("User-agent: BadBot".to_string());
    lines.push("Disallow: /".to_string());
    lines
}

/// Generate unique URL strings.
fn generate_urls(count: usize) -> Vec<CaseInsensitiveString> {
    (0..count)
        .map(|i| CaseInsensitiveString::from(format!("https://example.com/page/{i}")))
        .collect()
}

// ---------------------------------------------------------------------------
// Group 1: HTML cleaning
// ---------------------------------------------------------------------------

fn bench_html_cleaning(c: &mut Criterion) {
    let mut group = c.benchmark_group("html-cleaning");

    let sizes = [
        ("small-1KB", 8),
        ("medium-50KB", 400),
        ("large-200KB", 1600),
    ];

    for (label, count) in &sizes {
        let html = generate_html(*count);

        group.bench_with_input(BenchmarkId::new("clean_html_base", label), &html, |b, h| {
            b.iter(|| clean_html_base(black_box(h)))
        });
        group.bench_with_input(BenchmarkId::new("clean_html_slim", label), &html, |b, h| {
            b.iter(|| clean_html_slim(black_box(h)))
        });
        group.bench_with_input(BenchmarkId::new("clean_html_full", label), &html, |b, h| {
            b.iter(|| clean_html_full(black_box(h)))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 2: URL processing
// ---------------------------------------------------------------------------

fn bench_url_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("url-processing");

    let base = parse_absolute_url("https://example.com/some/path/").unwrap();

    group.bench_function("convert_abs_path/relative", |b| {
        b.iter(|| convert_abs_path(black_box(&base), black_box("/products/item-42?ref=home")))
    });

    group.bench_function("convert_abs_path/absolute", |b| {
        b.iter(|| {
            convert_abs_path(
                black_box(&base),
                black_box("https://other.example.org/landing"),
            )
        })
    });

    group.bench_function("flip_http_https", |b| {
        b.iter(|| flip_http_https(black_box("https://example.com/page/123")))
    });

    group.bench_function("prepare_url", |b| {
        b.iter(|| prepare_url(black_box("example.com/page")))
    });

    group.bench_function("get_last_segment", |b| {
        b.iter(|| get_last_segment(black_box("/api/v2/users/profile")))
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 3: Interner / ListBucket
// ---------------------------------------------------------------------------

fn bench_interner(c: &mut Criterion) {
    let mut group = c.benchmark_group("interner");

    let urls_10k = generate_urls(10_000);

    group.bench_function("insert/10k", |b| {
        b.iter(|| {
            let mut bucket = ListBucket::new();
            for u in &urls_10k {
                bucket.insert(black_box(u.clone()));
            }
            bucket
        })
    });

    // Pre-populate a bucket for lookup benchmarks
    let mut populated = ListBucket::new();
    for u in &urls_10k {
        populated.insert(u.clone());
    }

    group.bench_function("contains_hit/10k", |b| {
        let needle = &urls_10k[5000];
        b.iter(|| populated.contains(black_box(needle)))
    });

    group.bench_function("contains_miss/10k", |b| {
        let needle = CaseInsensitiveString::from("https://example.com/page/not-exists-99999");
        b.iter(|| populated.contains(black_box(&needle)))
    });

    group.bench_function("extend_links/1k_new_vs_10k_visited", |b| {
        b.iter_batched(
            || {
                // Setup: clone the populated bucket, prepare new links
                let bucket = populated.clone();
                let new_links: HashSet<CaseInsensitiveString> = (10_000..11_000)
                    .map(|i| CaseInsensitiveString::from(format!("https://example.com/page/{i}")))
                    .collect();
                let pending: HashSet<CaseInsensitiveString> = HashSet::new();
                (bucket, pending, new_links)
            },
            |(mut bucket, mut pending, new_links)| {
                bucket.extend_links(black_box(&mut pending), black_box(new_links));
                (bucket, pending)
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 4: Robots.txt parser
// ---------------------------------------------------------------------------

fn bench_robotparser(c: &mut Criterion) {
    let mut group = c.benchmark_group("robotparser");

    let lines_100 = generate_robots_txt(100);

    group.bench_function("parse/100_lines", |b| {
        b.iter(|| {
            let mut parser = RobotFileParser::new();
            parser.parse(black_box(&lines_100));
            parser
        })
    });

    // Pre-parse for can_fetch benchmarks
    let mut parsed = RobotFileParser::new();
    parsed.modified();
    parsed.parse(&lines_100);

    group.bench_function("can_fetch/allowed", |b| {
        b.iter(|| {
            parsed.can_fetch(
                black_box("spider"),
                black_box("https://example.com/public/1/page"),
            )
        })
    });

    group.bench_function("can_fetch/blocked", |b| {
        b.iter(|| {
            parsed.can_fetch(
                black_box("spider"),
                black_box("https://example.com/private/0/secret"),
            )
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 5: Trie
// ---------------------------------------------------------------------------

fn bench_trie(c: &mut Criterion) {
    let mut group = c.benchmark_group("trie");

    let paths: Vec<String> = (0..1000)
        .map(|i| format!("/section/{}/page/{}", i % 50, i))
        .collect();

    group.bench_function("insert/1000_paths", |b| {
        b.iter(|| {
            let mut trie = Trie::new();
            for (i, p) in paths.iter().enumerate() {
                trie.insert(black_box(p), black_box(i));
            }
            trie
        })
    });

    // Pre-populate for search benchmarks
    let mut populated_trie: Trie<usize> = Trie::new();
    for (i, p) in paths.iter().enumerate() {
        populated_trie.insert(p, i);
    }

    group.bench_function("search/hit", |b| {
        b.iter(|| populated_trie.search(black_box("/section/25/page/500")))
    });

    group.bench_function("search/miss", |b| {
        b.iter(|| populated_trie.search(black_box("/nonexistent/path/here")))
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 6: Antibot detection
// ---------------------------------------------------------------------------

fn bench_antibot(c: &mut Criterion) {
    let mut group = c.benchmark_group("antibot-detection");

    group.bench_function("detect_antibot_url/clean", |b| {
        b.iter(|| detect_antibot_from_url(black_box("https://example.com/products/shoes")))
    });

    group.bench_function("detect_antibot_url/cloudflare", |b| {
        b.iter(|| {
            detect_antibot_from_url(black_box(
                "https://example.com/cdn-cgi/challenge-platform/scripts/managed/init.js",
            ))
        })
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Group 7: CSS extraction (spider_utils)
// ---------------------------------------------------------------------------

fn bench_css_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("css-extraction");

    // Build selector map: 10 selectors
    let mut selector_map: HashMap<String, Vec<String>> = HashMap::new();
    let selectors = [
        ("titles", "h1, h2, h3"),
        ("links", "a[href]"),
        ("images", "img"),
        ("paragraphs", "p"),
        ("divs", "div.card"),
        ("scripts", "script[src]"),
        ("meta", "meta[name]"),
        ("lists", "ul, ol"),
        ("forms", "form"),
        ("nav", "nav a"),
    ];
    for (key, sel) in &selectors {
        selector_map.insert(key.to_string(), vec![sel.to_string()]);
    }

    group.bench_function("build_selectors/10_selectors", |b| {
        b.iter(|| {
            spider_utils::build_selectors_base::<String, String, Vec<String>>(black_box(
                selector_map.clone(),
            ))
        })
    });

    let doc_selectors =
        spider_utils::build_selectors_base::<String, String, Vec<String>>(selector_map.clone());
    let html_50kb = generate_html(400);

    group.bench_function("css_query_select_map/10_selectors_on_50kb", |b| {
        b.iter(|| spider_utils::css_query_select_map(black_box(&html_50kb), black_box(&doc_selectors)))
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Criterion harness
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_html_cleaning,
    bench_url_processing,
    bench_interner,
    bench_robotparser,
    bench_trie,
    bench_antibot,
    bench_css_extraction,
);
criterion_main!(benches);
