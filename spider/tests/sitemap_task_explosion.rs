use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use spider::tokio;
use spider::website::Website;

// ---------------------------------------------------------------------------
// Sitemap XML generators
// ---------------------------------------------------------------------------

fn single_sitemap_large(base: &str, n: usize) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for i in 0..n {
        xml.push_str(&format!("  <url><loc>{}/page/{}</loc></url>\n", base, i));
    }
    xml.push_str("</urlset>\n");
    xml
}

// ---------------------------------------------------------------------------
// Minimal HTTP server
// ---------------------------------------------------------------------------

/// Start a simple HTTP server with a single large sitemap (no index).
fn start_single_sitemap_server(base: String, url_count: usize) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };

            let base = base.clone();
            let url_count = url_count;
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let request = String::from_utf8_lossy(&buf);

                if let Some(path) = request.lines().next().and_then(|l| l.split(' ').nth(1)) {
                    let response = if path == "/sitemap.xml" {
                        single_sitemap_large(&base, url_count)
                    } else if path.starts_with("/page/") {
                        // Slow response to keep HTTP requests in-flight
                        thread::sleep(Duration::from_millis(50));
                        "<html><body>ok</body></html>".to_string()
                    } else if path == "/robots.txt" || path == "/" {
                        format!("User-agent: *\nAllow: /\nSitemap: {}/sitemap.xml\n", base)
                    } else {
                        "not found".to_string()
                    };

                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        response.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(response.as_bytes());
                }
            });
        }
    });

    port
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

const URL_COUNT: usize = 2000; // Single sitemap with many URLs

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crawl_sitemap_semaphore_bounding() {
    let _ = env_logger::builder().is_test(true).try_init();

    // ── Start server ──
    let base_port = start_single_sitemap_server("http://127.0.0.1".to_string(), URL_COUNT);

    println!(
        "=== Test server at 127.0.0.1:{} — {} URLs in single sitemap ===",
        base_port, URL_COUNT,
    );

    // ── Spider config: concurrency = 2 ──
    let base = format!("http://127.0.0.1:{}", base_port);
    let mut website = Website::new(&base);
    website
        .configuration
        .with_respect_robots_txt(false)
        .with_delay(0)
        .with_request_timeout(Some(Duration::from_secs(30)))
        .with_concurrency_limit(Some(2));

    website.subscribe(1024);

    // ── Sample alive-task count every 1ms while crawling ──
    // With unbounded spawning, we'd see many tasks queued waiting on tx.reserve().
    // With semaphore gating, tasks wait on the semaphore BEFORE spawning.
    let peak = Arc::new(AtomicU64::new(0));

    let peak_c = Arc::clone(&peak);
    let sampler = tokio::spawn(async move {
        let start = std::time::Instant::now();
        loop {
            let n = tokio::runtime::Handle::current()
                .metrics()
                .num_alive_tasks() as u64;
            peak_c.fetch_max(n, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(1)).await;
            if start.elapsed().as_secs() > 60 {
                break;
            }
        }
    });

    // Small delay to let sampler start
    tokio::time::sleep(Duration::from_millis(10)).await;

    // ── Crawl ──
    website.crawl_sitemap().await;

    // Wait for crawl to finish
    tokio::time::sleep(Duration::from_millis(200)).await;

    let peak_val = peak.load(Ordering::Relaxed);

    // ── Cleanup ──
    website.unsubscribe();
    sampler.abort();

    // ── Report ──
    println!("\n--- Results ---");
    println!("Peak alive tasks : {}", peak_val);
    println!("concurrency_limit: 2");
    println!("Total URLs       : {}", URL_COUNT);

    // ── Assertion ──
    // With unbounded spawning, thousands of tasks could be alive simultaneously.
    // With semaphore-gated spawning, only ~N tasks (where N = concurrency_limit + buffer)
    // should be alive at once.
    //
    // Note: tower's ConcurrencyLimitLayer also limits in-flight HTTP requests,
    // but the original code spawns ALL tasks immediately regardless.
    // The fix makes task spawning respect concurrency_limit via semaphore.
    assert!(
        peak_val <= 30,
        "Peak {} tasks exceeds 30 with concurrency_limit=2. Tasks may be spawning unboundedly instead of waiting for semaphore permits! (Expected ~2-8 tasks for {} URLs with slow responses)",
        peak_val,
        URL_COUNT,
    );
}
