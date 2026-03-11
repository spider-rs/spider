use std::io::Write;
use std::net::TcpListener;
use std::time::{Duration, Instant};

/// Spin up a local TCP server that sends partial HTTP body then stalls (simulating tarpit / antibot).
fn start_tarpit_server(partial_body: &'static [u8], content_length: usize) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();

            // Read the request (consume it so reqwest doesn't hang)
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).unwrap();

            // Send HTTP headers with Content-Length larger than what we'll send
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\n\r\n",
                content_length
            );
            stream.write_all(headers.as_bytes()).unwrap();
            stream.write_all(partial_body).unwrap();
            stream.flush().unwrap();

            // Now stall forever (tarpit). The idle timeout should kick in.
            std::thread::sleep(Duration::from_secs(120));
        }
    });

    port
}

use std::io::Read;

#[tokio::test]
async fn chunk_idle_timeout_returns_partial_content() {
    // Set a short idle timeout for testing
    std::env::set_var("SPIDER_CHUNK_IDLE_TIMEOUT_SECS", "1");

    let partial_html = b"<html><body><h1>Hello</h1>";
    let port = start_tarpit_server(partial_html, 10_000); // Claim 10KB but only send 25 bytes

    let client = spider::reqwest::Client::builder()
        .timeout(Duration::from_secs(30)) // Overall timeout is long
        .build()
        .unwrap();

    let url = format!("http://127.0.0.1:{}/", port);

    let start = Instant::now();
    let response = client.get(&url).send().await.unwrap();

    let page_response = spider::utils::handle_response_bytes(response, &url, false).await;
    let elapsed = start.elapsed();

    // Should complete in ~1-2 seconds (idle timeout), NOT 30 seconds (request timeout)
    assert!(
        elapsed < Duration::from_secs(5),
        "should have timed out quickly, took {:?}",
        elapsed
    );

    // Partial content should be preserved
    let content = page_response.content.expect("should have partial content");
    assert_eq!(
        content.as_slice(),
        partial_html,
        "partial body should be returned"
    );

    // Status code should reflect the original 200
    assert_eq!(page_response.status_code, spider::reqwest::StatusCode::OK);
}

#[tokio::test]
async fn chunk_idle_timeout_normal_response_unaffected() {
    // Set idle timeout
    std::env::set_var("SPIDER_CHUNK_IDLE_TIMEOUT_SECS", "2");

    let full_body = b"<html><body><p>Complete page</p></body></html>";
    let port = start_normal_server(full_body);

    let client = spider::reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let url = format!("http://127.0.0.1:{}/", port);
    let response = client.get(&url).send().await.unwrap();

    let page_response = spider::utils::handle_response_bytes(response, &url, false).await;

    let content = page_response.content.expect("should have content");
    assert_eq!(content.as_slice(), full_body);
    assert_eq!(page_response.status_code, spider::reqwest::StatusCode::OK);
}

/// Normal server that sends a complete response immediately.
fn start_normal_server(body: &'static [u8]) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();

            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf).unwrap();

            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).unwrap();
            stream.write_all(body).unwrap();
            // Connection closes naturally - no stalling
        }
    });

    port
}

#[tokio::test]
async fn chunk_idle_timeout_disabled_with_zero() {
    // Disable idle timeout
    std::env::set_var("SPIDER_CHUNK_IDLE_TIMEOUT_SECS", "0");

    // With timeout disabled and a tarpit, the request should hit the reqwest
    // overall timeout instead. We use a very short reqwest timeout to test this.
    let partial_html = b"<html>partial";
    let port = start_tarpit_server(partial_html, 50_000);

    let client = spider::reqwest::Client::builder()
        .timeout(Duration::from_secs(2)) // Short overall timeout
        .build()
        .unwrap();

    let url = format!("http://127.0.0.1:{}/", port);
    let start = Instant::now();
    let response = client.get(&url).send().await.unwrap();

    let page_response = spider::utils::handle_response_bytes(response, &url, false).await;
    let elapsed = start.elapsed();

    // Without chunk idle timeout, it waits for reqwest overall timeout (~2s)
    // The partial content should still be salvaged via the Err(e) => break path
    assert!(
        elapsed >= Duration::from_secs(1),
        "without idle timeout, should wait for reqwest timeout"
    );

    // Content may or may not be present depending on how reqwest handles the timeout,
    // but the function should not panic
    let content = page_response.content.unwrap_or_default();
    // Partial data may have been collected before the error
    if !content.is_empty() {
        assert_eq!(content.as_slice(), partial_html);
    }
}
