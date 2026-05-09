//! cargo run --release --example chrome_remote_logrocket --features="spider/sync spider/chrome spider/chrome_intercept"
//!
//! Single-page Chrome fetch of https://logrocket.com/careers via remote CDP.
//! Runs the same fetch under several wait/intercept strategies back-to-back to
//! reproduce the 60s timeout seen in the private crawler stack.
//!
//! Modes:
//!   - none            : default chrome (no intercept overrides, no wait)
//!   - idle_network    : strict network idle (~500ms gap, 30s ceiling)
//!   - almost_idle     : partial idle (30s ceiling)
//!   - private_repro   : mirrors PRIVATE_CODE/crawler setup.rs:
//!                       block_visuals/js/analytics/ads/stylesheets + rng idle
//!                       waits + 30s request_timeout + lr-marketing-js whitelist
//!   - private_no_js   : same as private_repro but block_javascript=false
//!   - private_no_wl   : same as private_repro but no analytics whitelist
//!
//! Prereq: remote Chrome on 127.0.0.1:9222 (e.g. the headless-browser project).

extern crate spider;

use spider::configuration::{WaitForIdleNetwork, WaitForSelector};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::website::Website;
use std::time::{Duration, Instant};

const TARGET: &str = "https://logrocket.com/careers";
const CDP_URL: &str = "http://127.0.0.1:9222/json/version";
const WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const LR_WHITELIST: &str = "https://storage.googleapis.com/lr-marketing-js/lr-web-analytics/";

#[derive(Clone, Copy)]
enum Mode {
    None,
    IdleNetwork,
    AlmostIdle,
    PrivateRepro,
    PrivateNoJs,
    PrivateNoWhitelist,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Mode::None => "none",
            Mode::IdleNetwork => "idle_network",
            Mode::AlmostIdle => "almost_idle",
            Mode::PrivateRepro => "private_repro",
            Mode::PrivateNoJs => "private_no_js",
            Mode::PrivateNoWhitelist => "private_no_wl",
        }
    }
}

fn private_intercept(block_js: bool, whitelist: bool) -> RequestInterceptConfiguration {
    let mut intercept = RequestInterceptConfiguration::new(true);
    intercept.block_visuals = true;
    intercept.block_stylesheets = true;
    intercept.block_javascript = block_js;
    intercept.block_analytics = true;
    intercept.block_ads = true;
    if whitelist {
        intercept.whitelist_patterns = Some(vec![LR_WHITELIST.into()]);
    }
    intercept
}

fn apply_rng_idle_wait(w: &mut Website) {
    // Match private setup.rs:rng_idle_wait
    let almost_idle_ms = fastrand_u64(2_000..3_500);
    let idle_network_ms = fastrand_u64(2_000..4_000);
    let body_timeout_ms = fastrand_u64(250..750);
    w.with_wait_for_almost_idle_network0(Some(WaitForIdleNetwork::new(Some(
        Duration::from_millis(almost_idle_ms),
    ))))
    .with_wait_for_idle_network0(Some(WaitForIdleNetwork::new(Some(Duration::from_millis(
        idle_network_ms,
    )))))
    .with_wait_for_idle_dom(Some(WaitForSelector::new(
        Some(Duration::from_millis(body_timeout_ms)),
        "body".into(),
    )));
    eprintln!(
        "  [rng_idle] almost={almost_idle_ms}ms idle0={idle_network_ms}ms dom={body_timeout_ms}ms (total~{}ms)",
        almost_idle_ms + idle_network_ms + body_timeout_ms
    );
}

fn fastrand_u64(range: std::ops::Range<u64>) -> u64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    range.start + (nanos % (range.end - range.start))
}

async fn run(mode: Mode) -> (Duration, usize, u16) {
    let mut builder = Website::new(TARGET)
        .with_limit(1)
        .with_stealth(true)
        .with_chrome_connection(Some(CDP_URL.into()))
        .build()
        .unwrap();

    match mode {
        Mode::None => {
            builder.with_chrome_intercept(RequestInterceptConfiguration::new(true));
        }
        Mode::IdleNetwork => {
            builder
                .with_chrome_intercept(RequestInterceptConfiguration::new(true))
                .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(WAIT_TIMEOUT))));
        }
        Mode::AlmostIdle => {
            builder
                .with_chrome_intercept(RequestInterceptConfiguration::new(true))
                .with_wait_for_almost_idle_network0(Some(WaitForIdleNetwork::new(Some(
                    WAIT_TIMEOUT,
                ))));
        }
        Mode::PrivateRepro => {
            builder
                .with_chrome_intercept(private_intercept(true, true))
                .with_request_timeout(Some(REQUEST_TIMEOUT));
            apply_rng_idle_wait(&mut builder);
        }
        Mode::PrivateNoJs => {
            builder
                .with_chrome_intercept(private_intercept(false, true))
                .with_request_timeout(Some(REQUEST_TIMEOUT));
            apply_rng_idle_wait(&mut builder);
        }
        Mode::PrivateNoWhitelist => {
            builder
                .with_chrome_intercept(private_intercept(true, false))
                .with_request_timeout(Some(REQUEST_TIMEOUT));
            apply_rng_idle_wait(&mut builder);
        }
    }

    let mut rx = builder.subscribe(4);
    let collector = tokio::spawn(async move {
        let mut html_bytes = 0usize;
        let mut status = 0u16;
        while let Ok(page) = rx.recv().await {
            html_bytes = page.get_html_bytes_u8().len();
            status = page.status_code.as_u16();
        }
        (html_bytes, status)
    });

    let start = Instant::now();
    builder.crawl().await;
    builder.unsubscribe();
    let elapsed = start.elapsed();

    let (bytes, status) = collector.await.unwrap();
    (elapsed, bytes, status)
}

#[tokio::main]
async fn main() {
    env_logger::init();

    println!("Target: {TARGET}");
    println!("Remote: {CDP_URL}");
    println!();

    let modes = [
        Mode::None,
        Mode::IdleNetwork,
        Mode::AlmostIdle,
        Mode::PrivateRepro,
        Mode::PrivateNoJs,
        Mode::PrivateNoWhitelist,
    ];
    let mut results = Vec::with_capacity(modes.len());

    for mode in modes {
        let label = mode.label();
        eprintln!(">> running {label}");
        let (elapsed, bytes, status) = run(mode).await;
        println!("{label:<16} elapsed={elapsed:>10?}  status={status}  html={bytes}B");
        results.push((label, elapsed));
    }

    println!();
    if let Some((_, baseline)) = results.first().copied() {
        println!("Deltas vs '{}':", results[0].0);
        for (label, elapsed) in &results[1..] {
            let delta = elapsed.as_secs_f64() - baseline.as_secs_f64();
            println!("  {label:<16} {delta:+.3}s");
        }
    }
}
