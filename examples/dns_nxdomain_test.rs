//! cargo run --example dns_nxdomain_test
//!
//! Smoke test for proxy / NXDOMAIN classification: hits a likely-dead host and
//! a known-good host, prints elapsed + status + retry flag for each.

use spider::tokio;
use spider::website::Website;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    for url in &["https://midwestrodding.com/", "https://example.com/"] {
        let mut w = Website::new(url);
        w.with_limit(1).with_respect_robots_txt(false);
        w.configuration.request_timeout = Some(std::time::Duration::from_secs(10));
        let start = std::time::Instant::now();
        w.scrape().await;
        let elapsed = start.elapsed();
        let pages = w.get_pages();
        match pages {
            Some(ps) if !ps.is_empty() => {
                for p in ps.iter() {
                    println!(
                        "[{:.2}s] {} -> status={} html_len={} should_retry={}",
                        elapsed.as_secs_f32(),
                        p.get_url(),
                        p.status_code,
                        p.get_html_bytes_u8().len(),
                        p.should_retry,
                    );
                }
            }
            _ => println!("[{:.2}s] {} -> no pages", elapsed.as_secs_f32(), url),
        }
    }
}
