//! cargo run --example chrome_proxy_dns_shortcircuit --features "chrome spider_cloud"
//!
//! Verifies chrome short-circuits NXDOMAIN through a proxy without extra
//! requests. Reads an HTTP proxy URL from .env (e.g. EVOMI_UNLIMITED_HTTP),
//! starts a local CONNECT-injecting forwarder on 127.0.0.1:<port> so chrome
//! sees an auth-less proxy, then crawls https://www.hello.plantbid.com/ and
//! prints status_code, should_retry, and any extracted chrome errorCode.
//!
//! Required: PROXY_VAR=<name in .env> OR PROXY_URL=http://user:pass@host:port.
//! Optional: TARGET_URL=https://example.com (default www.hello.plantbid.com).
//!
//! Set PRINT_HTML_TAIL=1 to dump the last 4KB of the page for inspection.
//!
//! The forwarder also intentionally short-circuits CONNECT to
//! `127.0.0.1:0` so you can test the no-proxy local DNS path with
//! TARGET_URL=http://localhost:0 — irrelevant for the default case.

extern crate spider;

use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::tokio;
use spider::tokio::io::{AsyncReadExt, AsyncWriteExt};
use spider::tokio::net::{TcpListener, TcpStream};
use spider::website::Website;
use std::collections::HashMap;
use std::fs;
use std::time::Duration;

fn base64_encode(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | input[i + 2] as u32;
        out.push(T[((n >> 18) & 0x3F) as usize] as char);
        out.push(T[((n >> 12) & 0x3F) as usize] as char);
        out.push(T[((n >> 6) & 0x3F) as usize] as char);
        out.push(T[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(T[((n >> 18) & 0x3F) as usize] as char);
        out.push(T[((n >> 12) & 0x3F) as usize] as char);
        out.push_str("==");
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(T[((n >> 18) & 0x3F) as usize] as char);
        out.push(T[((n >> 12) & 0x3F) as usize] as char);
        out.push(T[((n >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}

#[derive(Clone, Debug)]
struct UpstreamProxy {
    host: String,
    port: u16,
    auth_header: Option<String>,
}

fn parse_proxy(url: &str) -> Option<UpstreamProxy> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let (auth, hostport) = match rest.split_once('@') {
        Some((a, hp)) => (Some(a), hp),
        None => (None, rest),
    };
    let hostport = hostport.split('/').next().unwrap_or(hostport);
    let (host, port_s) = hostport.split_once(':')?;
    let port: u16 = port_s.parse().ok()?;
    let auth_header = auth.map(|a| {
        let b64 = base64_encode(a.as_bytes());
        format!("Proxy-Authorization: Basic {b64}\r\n")
    });
    Some(UpstreamProxy {
        host: host.to_string(),
        port,
        auth_header,
    })
}

fn load_env(name: &str) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    let mut cur = std::env::current_dir().ok();
    while let Some(dir) = cur.clone() {
        let p = dir.join(name);
        if let Ok(body) = fs::read_to_string(&p) {
            eprintln!("[env] loaded {}", p.display());
            for line in body.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    let v = v.trim().trim_matches('"').trim_matches('\'');
                    if v.is_empty() {
                        continue;
                    }
                    out.entry(k.trim().to_string())
                        .or_insert_with(|| v.to_string());
                }
            }
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
    out
}

async fn handle_client(mut client: TcpStream, upstream: UpstreamProxy) -> std::io::Result<()> {
    let mut buf = vec![0u8; 8192];
    let mut filled = 0usize;
    loop {
        let n = client.read(&mut buf[filled..]).await?;
        if n == 0 {
            return Ok(());
        }
        filled += n;
        if buf[..filled].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if filled == buf.len() {
            return Ok(());
        }
    }

    let header_end = buf[..filled]
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| p + 4)
        .unwrap_or(filled);
    let request = std::str::from_utf8(&buf[..header_end]).unwrap_or("");
    let first_line = request.lines().next().unwrap_or("");
    let is_connect = first_line.starts_with("CONNECT ");

    let mut up = TcpStream::connect((upstream.host.as_str(), upstream.port)).await?;

    if is_connect {
        // Preserve chrome's original CONNECT request, inject our
        // Proxy-Authorization header right before the terminating CRLFCRLF.
        let mut out = Vec::with_capacity(header_end + 256);
        let body = &buf[..header_end];
        let head_end_idx = header_end - 4; // points at first \r\n\r\n
        out.extend_from_slice(&body[..head_end_idx]);
        out.extend_from_slice(b"\r\n");
        if let Some(a) = &upstream.auth_header {
            out.extend_from_slice(a.as_bytes());
        }
        out.extend_from_slice(b"\r\n");
        eprintln!(
            "[forwarder] sending CONNECT ({} bytes), auth_present={}",
            out.len(),
            upstream.auth_header.is_some()
        );
        up.write_all(&out).await?;

        let mut resp = vec![0u8; 8192];
        let mut rfilled = 0usize;
        loop {
            let n = up.read(&mut resp[rfilled..]).await?;
            if n == 0 {
                break;
            }
            rfilled += n;
            if resp[..rfilled].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
            if rfilled == resp.len() {
                break;
            }
        }
        let upstream_status = std::str::from_utf8(&resp[..rfilled])
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        eprintln!("[forwarder] upstream CONNECT response: {upstream_status}");
        client.write_all(&resp[..rfilled]).await?;
    } else {
        let mut out = String::from(request);
        if let Some(a) = &upstream.auth_header {
            let insert_at = out.find("\r\n\r\n").unwrap_or(out.len());
            out.insert_str(insert_at, &format!("\r\n{}", a.trim_end_matches("\r\n")));
        }
        up.write_all(out.as_bytes()).await?;
        if header_end < filled {
            up.write_all(&buf[header_end..filled]).await?;
        }
    }

    let _ = spider::tokio::io::copy_bidirectional(&mut client, &mut up).await;
    Ok(())
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let env_map = load_env(".env");
    let proxy_url = std::env::var("PROXY_URL").ok().or_else(|| {
        std::env::var("PROXY_VAR")
            .ok()
            .and_then(|v| env_map.get(&v).cloned())
    });
    let proxy_url = match proxy_url {
        Some(u) => u,
        None => {
            eprintln!(
                "Set PROXY_URL=http://user:pass@host:port OR PROXY_VAR=<name of var in .env>"
            );
            std::process::exit(2);
        }
    };
    let upstream = parse_proxy(&proxy_url).expect("invalid proxy URL");
    eprintln!(
        "[forwarder] upstream = {}:{} (auth: {})",
        upstream.host,
        upstream.port,
        upstream.auth_header.is_some()
    );

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let local_addr = listener.local_addr()?;
    eprintln!("[forwarder] listening on http://{local_addr}");

    let up_clone = upstream.clone();
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((sock, _)) => {
                    let up = up_clone.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(sock, up).await {
                            eprintln!("[forwarder] client error: {e}");
                        }
                    });
                }
                Err(e) => {
                    eprintln!("[forwarder] accept error: {e}");
                    break;
                }
            }
        }
    });

    let target = std::env::var("TARGET_URL")
        .unwrap_or_else(|_| "https://www.hello.plantbid.com/".to_string());
    let local_proxy = format!("http://{local_addr}");

    let mut website = Website::new(&target)
        .with_limit(1)
        .with_respect_robots_txt(false)
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_proxies(Some(vec![local_proxy.clone()]))
        .build()
        .unwrap();
    website.configuration.request_timeout = Some(Duration::from_secs(20));

    let mut rx = website.subscribe(8);
    let sub_handle = tokio::spawn(async move {
        let mut seen = Vec::new();
        while let Ok(page) = rx.recv().await {
            let html = page.get_html_bytes_u8();
            let chrome_error = spider::page::is_chrome_error_page(html);
            let error_code = if chrome_error {
                spider::page::extract_chrome_error_code(html).map(|s| s.to_string())
            } else {
                None
            };
            println!(
                "[subscribe] {} -> status={} should_retry={} html_len={} chrome_error_page={} errorCode={:?}",
                page.get_url(),
                page.status_code,
                page.should_retry,
                html.len(),
                chrome_error,
                error_code,
            );
            seen.push(page.get_url().to_string());
        }
        seen
    });

    let start = std::time::Instant::now();
    website.scrape().await;
    let elapsed = start.elapsed();
    website.unsubscribe();
    let _ = sub_handle.await;

    let pages = website.get_pages();
    match pages {
        Some(ps) if !ps.is_empty() => {
            for p in ps.iter() {
                let html = p.get_html_bytes_u8();
                let chrome_error = spider::page::is_chrome_error_page(html);
                let error_code = if chrome_error {
                    spider::page::extract_chrome_error_code(html).map(|s| s.to_string())
                } else {
                    None
                };
                println!(
                    "[{:.2}s] {} -> status={} should_retry={} html_len={} chrome_error_page={} errorCode={:?}",
                    elapsed.as_secs_f32(),
                    p.get_url(),
                    p.status_code,
                    p.should_retry,
                    html.len(),
                    chrome_error,
                    error_code,
                );
                if std::env::var("PRINT_HTML_TAIL").ok().as_deref() == Some("1") && !html.is_empty()
                {
                    let tail_start = html.len().saturating_sub(4096);
                    let tail = String::from_utf8_lossy(&html[tail_start..]);
                    println!("---- last 4KB ----\n{tail}\n---- end ----");
                }
            }
        }
        _ => println!("[{:.2}s] {} -> no pages", elapsed.as_secs_f32(), target),
    }

    Ok(())
}
