//! Measurement test for picking a sane `chrome_first_byte_timeout`.
//!
//! Connects to a chrome backend (auto-launched locally or pointed at a
//! remote one via `CHROME_URL` env), navigates a list of URLs, and prints
//! per-URL timings for the FIRST `Network.responseReceived` and
//! `Network.dataReceived` events. Aggregates min / p50 / p90 / p99 / max
//! so you can pick `chrome_first_byte_timeout = p99 + headroom`.
//!
//! Live test — gated on `RUN_LIVE_TESTS=1`. Skipped otherwise.
//!
//! Run:
//!
//! ```sh
//! RUN_LIVE_TESTS=1 cargo test -p spider \
//!   --features "chrome" --test chrome_first_byte_timing -- --nocapture
//! ```
//!
//! Or against a remote chrome:
//!
//! ```sh
//! RUN_LIVE_TESTS=1 CHROME_URL=ws://...:9222/devtools/browser/... \
//!   cargo test -p spider --features "chrome" \
//!   --test chrome_first_byte_timing -- --nocapture
//! ```

#![cfg(feature = "chrome")]

use std::env;
use std::time::{Duration, Instant};

fn run_live_tests() -> bool {
    matches!(
        env::var("RUN_LIVE_TESTS")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// URLs to probe. Mix of fast static, JS-heavy, and intentionally slow
/// targets so the percentile spread reflects real-world variance.
const TARGETS: &[&str] = &[
    // ── Static / fast ──
    "https://example.com/",
    "https://www.iana.org/",
    "https://www.crawler-test.com/",
    "https://www.crawler-test.com/javascript/render_only_after_js",
    "https://www.crawler-test.com/status_codes/status_200",
    // ── Docs / dev ──
    "https://www.rust-lang.org/",
    "https://docs.rs/spider",
    "https://github.com/",
    "https://stackoverflow.com/",
    // ── News / heavy CDN ──
    "https://www.bbc.com/",
    "https://www.nytimes.com/",
    "https://www.cnn.com/",
    "https://www.reddit.com/",
    // ── E-commerce (anti-bot likely) ──
    "https://www.amazon.com/",
    "https://www.ebay.com/",
    "https://www.shopify.com/",
    // ── SPAs ──
    "https://twitter.com/",
    "https://www.linkedin.com/",
    // ── International ──
    "https://www.bbc.co.uk/",
    "https://www.spiegel.de/",
    "https://www.lemonde.fr/",
    // ── Anti-bot / Cloudflare-fronted ──
    "https://www.cloudflare.com/",
    "https://www.discord.com/",
    // ── Wikipedia (high-volume reference) ──
    "https://en.wikipedia.org/wiki/Web_crawler",
    "https://en.wikipedia.org/wiki/HTTP",
];

#[derive(Default)]
struct Stats {
    response_first_ms: Vec<u128>,
    data_first_ms: Vec<u128>,
}

fn pct(samples: &mut [u128], p: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let idx = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[idx.min(samples.len() - 1)]
}

fn report(label: &str, samples: &mut Vec<u128>) {
    if samples.is_empty() {
        println!("  {label}: n=0 (no events)");
        return;
    }
    let n = samples.len();
    let min = *samples.iter().min().unwrap();
    let max = *samples.iter().max().unwrap();
    let p50 = pct(samples, 0.50);
    let p90 = pct(samples, 0.90);
    let p99 = pct(samples, 0.99);
    println!("  {label}: n={n}  min={min}ms  p50={p50}ms  p90={p90}ms  p99={p99}ms  max={max}ms");
}

#[tokio::test]
async fn measure_chrome_first_byte_timing() {
    if !run_live_tests() {
        eprintln!("skipping: set RUN_LIVE_TESTS=1 to run");
        return;
    }

    use chromiumoxide::cdp::browser_protocol::network::{EventDataReceived, EventResponseReceived};
    use spider::tokio_stream::StreamExt;

    // Launch / connect chrome via spider's existing config plumbing —
    // honors CHROME_URL env, single-URL connection, or local launch.
    let cfg = spider::configuration::Configuration::default();
    let (browser, _handle, _ctx, _dead, connected_url) =
        match spider::features::chrome::launch_browser(&cfg, &None).await {
            Some(t) => t,
            None => {
                eprintln!("skipping: failed to launch / connect chrome");
                return;
            }
        };
    println!(
        "[chrome] connected via {}",
        connected_url.as_deref().unwrap_or("local launch")
    );

    let mut stats = Stats::default();

    for url in TARGETS {
        // Fresh tab per URL.
        let page = match browser.new_page("about:blank").await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[skip] tab create failed for {url}: {e:?}");
                continue;
            }
        };

        let resp_listener = match page.event_listener::<EventResponseReceived>().await {
            Ok(l) => l,
            Err(_) => continue,
        };
        let data_listener = match page.event_listener::<EventDataReceived>().await {
            Ok(l) => l,
            Err(_) => continue,
        };

        let start = Instant::now();
        let url_owned = (*url).to_string();
        let nav = tokio::spawn({
            let page = page.clone();
            async move {
                // Navigate; ignore the result — we care about events.
                let _ = tokio::time::timeout(Duration::from_secs(30), page.goto(url_owned)).await;
            }
        });

        let resp_first = async {
            let mut l = resp_listener;
            tokio::time::timeout(Duration::from_secs(15), l.next())
                .await
                .ok()
                .flatten()
                .map(|_| start.elapsed().as_millis())
        };
        let data_first = async {
            let mut l = data_listener;
            tokio::time::timeout(Duration::from_secs(15), l.next())
                .await
                .ok()
                .flatten()
                .map(|_| start.elapsed().as_millis())
        };
        let (rms, dms) = tokio::join!(resp_first, data_first);
        let _ = nav.await;

        if let Some(ms) = rms {
            stats.response_first_ms.push(ms);
        }
        if let Some(ms) = dms {
            stats.data_first_ms.push(ms);
        }
        println!(
            "[probe] {url}  resp={:?}ms  data={:?}ms",
            rms.map(|v| v as i64).unwrap_or(-1),
            dms.map(|v| v as i64).unwrap_or(-1)
        );

        let _ = page.close().await;
    }

    println!(
        "\n=== first-byte timings across {} URL(s) ===",
        TARGETS.len()
    );
    report("Network.responseReceived", &mut stats.response_first_ms);
    report("Network.dataReceived    ", &mut stats.data_first_ms);
    println!(
        "\nRecommendation: chrome_first_byte_timeout ≈ p99 × 2 + 1000ms headroom.\n\
         Add jitter ≈ 20–30%% of the base via with_chrome_first_byte_timeout_jitter\n\
         to avoid thundering-herd rotation on shared backends."
    );
}
