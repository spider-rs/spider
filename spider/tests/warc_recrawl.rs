//! Crawling the same `Website` more than once with WARC output enabled must
//! keep one writer for the configured path, not start another per crawl.
#![cfg(feature = "warc")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use spider::tokio;
use spider::utils::warc::WarcConfig;
use spider::website::Website;

/// Three linked pages on 127.0.0.1: `/` links to `/a` and `/b`.
fn start_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let request = String::from_utf8_lossy(&buf);
                let path = request
                    .lines()
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("/");
                let body = match path {
                    "/" => "<html><body><a href=\"/a\">a</a><a href=\"/b\">b</a></body></html>",
                    "/a" | "/b" => "<html><body><a href=\"/\">home</a></body></html>",
                    _ => "",
                };
                let status = if body.is_empty() {
                    "404 Not Found"
                } else {
                    "200 OK"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recrawl_reuses_one_warc_writer() {
    const CRAWLS: usize = 3;
    const PAGES: usize = 3;

    let port = start_server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.warc");

    let mut website = Website::new(&format!("http://127.0.0.1:{port}/"));
    website.configuration.with_respect_robots_txt(false);
    website.configuration.with_warc(WarcConfig {
        path: path.to_string_lossy().into_owned(),
        write_warcinfo: true,
        ..Default::default()
    });

    let metrics = tokio::runtime::Handle::current().metrics();
    let mut alive = Vec::new();
    for _ in 0..CRAWLS {
        website.crawl().await;
        // Let the writer bridge drain this crawl's pages.
        tokio::time::sleep(Duration::from_millis(300)).await;
        alive.push(metrics.num_alive_tasks());
    }
    eprintln!("alive tasks after each crawl: {alive:?}");
    let reported = website.warc_record_count();
    eprintln!("warc_record_count(): {reported}");

    // Close the page channel so every writer bridge exits, then drop the
    // Website's own writer handle so the file writers flush and stop.
    website.unsubscribe();
    drop(website);
    tokio::time::sleep(Duration::from_millis(500)).await;

    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    let responses = text.matches("WARC-Type: response\r\n").count();
    let infos = text.matches("WARC-Type: warcinfo\r\n").count();
    let nul_bytes = bytes.iter().filter(|b| **b == 0).count();
    eprintln!(
        "file: {} bytes, {responses} response records, {infos} warcinfo, {nul_bytes} NUL bytes",
        bytes.len()
    );

    assert_eq!(
        alive.first(),
        alive.last(),
        "background tasks grew across crawls: {alive:?}"
    );
    assert_eq!(
        nul_bytes, 0,
        "file has holes from a truncated-then-overwritten writer"
    );
    assert_eq!(infos, 1);
    assert_eq!(
        reported as usize,
        infos + responses,
        "the Website's writer must be the one that wrote the file"
    );
    assert_eq!(
        responses,
        CRAWLS * PAGES,
        "each crawl's pages appended exactly once"
    );
}

/// After `unsubscribe()` ends the writer bridge, the next crawl must restart
/// the bridge on the same writer, so its pages still reach the file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recrawl_after_unsubscribe_keeps_writing() {
    let port = start_server();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.warc");

    let mut website = Website::new(&format!("http://127.0.0.1:{port}/"));
    website.configuration.with_respect_robots_txt(false);
    website.configuration.with_warc(WarcConfig {
        path: path.to_string_lossy().into_owned(),
        write_warcinfo: false,
        ..Default::default()
    });

    for _ in 0..2 {
        website.crawl().await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        website.unsubscribe();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    website.get_warc_writer().unwrap().flush().await.unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches("WARC-Type: response\r\n").count(), 6);
    assert_eq!(website.warc_record_count(), 6);
}
