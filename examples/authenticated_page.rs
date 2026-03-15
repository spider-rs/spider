//! Crawl a logged-in page with a reusable local browser profile and save rendered HTML plus
//! extracted text blocks.
//!
//! Recommended Ubuntu Desktop flow:
//! 1. Launch a dedicated persistent Chrome/Chromium profile and log in once:
//!
//! ```bash
//! PREPARE_PROFILE=1 \
//! CHROME_USER_DATA_DIR="$HOME/.local/share/spider/login-chrome-profile" \
//! cargo run --example authenticated_page \
//!   --features="spider/sync spider/cookies spider/chrome"
//! ```
//!
//! 2. After logging into the target site in that browser window, rerun the crawl:
//!
//! ```bash
//! CHROME_USER_DATA_DIR="$HOME/.local/share/spider/login-chrome-profile" \
//! cargo run --example authenticated_page \
//!   --features="spider/sync spider/cookies spider/chrome" \
//!   -- 'https://example.com/protected/page'
//! ```
//!
//! Legacy cookie mode is still available:
//!
//! ```bash
//! COOKIE='sessionid=...; csrftoken=...' \
//! cargo run --example authenticated_page \
//!   --features="spider/sync spider/cookies spider/chrome" \
//!   -- 'https://example.com/protected/page'
//! ```
//!
//! Optional environment variables:
//! - TARGET_URL: fallback target when no CLI arg is provided
//! - OUTPUT_DIR: directory for crawl artifacts, default `authenticated_page_output`
//! - OUTPUT_HTML: where to save the rendered HTML
//! - OUTPUT_JSON: where to save extracted text blocks
//! - DOWNLOAD_IMAGES: set to `0` to skip downloading image assets
//! - IMAGE_SELECTORS: selectors used to collect image URLs, default `img`
//! - USER_AGENT: override the browser-like user agent header
//! - ACCEPT_LANGUAGE_HEADER: override Accept-Language
//! - REFERER_URL: optional Referer header
//! - TITLE_SELECTORS: comma-separated CSS/XPath selectors
//! - CONTENT_SELECTORS: comma-separated CSS/XPath selectors
//! - WAIT_FOR_SELECTOR: CSS selector list to wait for before capture
//! - COOKIE: optional cookie string fallback
//! - CHROME_CONNECTION_URL: existing CDP endpoint like http://127.0.0.1:9222/json/version
//! - CHROME_USER_DATA_DIR: persistent local browser user-data-dir to reuse
//! - CHROME_PROFILE_DIR: Chrome profile directory inside the user-data-dir, default `Default`
//! - CHROME_BIN: browser binary path or executable name
//! - CHROME_DEBUGGING_PORT: local debugging port for the dedicated browser, default `9222`
//! - CHROME_START_URL: page to open when preparing or launching the dedicated browser
//! - CHROME_HEADLESS: set to `1` to launch the dedicated browser in headless mode
//! - CHROME_EXTRA_ARGS: extra Chromium args split by shell whitespace
//! - CHROME_PROXY: explicit proxy for the dedicated browser, like `http://127.0.0.1:7890`
//! - CHROME_INHERIT_PROXY: set to `1` to inherit terminal proxy env vars instead of clearing them
//! - PREPARE_PROFILE: set to `1` to launch/reuse the local browser and exit for manual login

extern crate env_logger;
extern crate spider;

use env_logger::Env;
use spider::client::header::{
    HeaderMap, HeaderValue, ACCEPT_LANGUAGE, CONTENT_TYPE, COOKIE as COOKIE_HEADER, REFERER,
    USER_AGENT,
};
use spider::configuration::{WaitForIdleNetwork, WaitForSelector};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::{HashMap, HashSet};
use spider::page::Page;
use spider::reqwest::{Client, Url};
use spider::tokio;
use spider::website::Website;
use spider_utils::build_selectors;
use std::env;
use std::fs;
use std::io::{Error, ErrorKind, Result};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

const DEFAULT_TARGET_URL: &str = "https://example.com/";
const DEFAULT_START_URL: &str = "https://example.com/";
const DEFAULT_WAIT_SELECTOR: &str = "body";
const DEFAULT_PROFILE_DIR: &str = "Default";
const DEFAULT_DEBUGGING_PORT: u16 = 9222;
const DEFAULT_OUTPUT_DIR: &str = "authenticated_page_output";

fn split_selectors(value: &str) -> HashSet<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn extraction_selectors() -> spider_utils::DocumentSelectors<String> {
    let title = env::var("TITLE_SELECTORS").unwrap_or_else(|_| "title,h1,h2".to_string());
    let content = env::var("CONTENT_SELECTORS").unwrap_or_else(|_| {
        [
            "main",
            "article",
            "[role='main']",
            ".content",
            ".main",
            ".article",
            ".post-content",
            ".entry-content",
            ".article-content",
            ".rich-text",
            ".RichContent",
            ".RichText",
            "body",
        ]
        .join(",")
    });
    let images = env::var("IMAGE_SELECTORS").unwrap_or_else(|_| "img".to_string());

    let mut selector_map: HashMap<String, HashSet<String>> = HashMap::new();
    selector_map.insert("title".to_string(), split_selectors(&title));
    selector_map.insert("content".to_string(), split_selectors(&content));
    selector_map.insert("images".to_string(), split_selectors(&images));

    build_selectors(selector_map)
}

fn json_error(err: serde_json::Error) -> Error {
    Error::other(err)
}

fn normalize_text(value: &str) -> String {
    value
        .replace('\u{200b}', " ")
        .replace('\u{00a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn compact_text_blocks(blocks: &mut Vec<String>, min_len: usize) {
    let mut cleaned: Vec<String> = Vec::new();

    for item in blocks.drain(..) {
        let normalized = normalize_text(&item);

        if normalized.len() < min_len {
            continue;
        }

        if cleaned.iter().any(|existing| existing == &normalized) {
            continue;
        }

        if cleaned
            .iter()
            .any(|existing| existing.len() > normalized.len() && existing.contains(&normalized))
        {
            continue;
        }

        cleaned.retain(|existing| {
            !(normalized.len() > existing.len() && normalized.contains(existing.as_str()))
        });
        cleaned.push(normalized);
    }

    cleaned.sort_by(|a, b| b.len().cmp(&a.len()));
    *blocks = cleaned;
}

fn post_process_extracted(extracted: &mut HashMap<String, Vec<String>>) {
    if let Some(title) = extracted.get_mut("title") {
        compact_text_blocks(title, 4);
        if title.len() > 2 {
            title.truncate(2);
        }
    }

    if let Some(content) = extracted.get_mut("content") {
        compact_text_blocks(content, 20);
        if content.len() > 5 {
            content.truncate(5);
        }
    }
}

fn env_truthy(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "y" | "on"
            )
        })
        .unwrap_or(false)
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }

    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    PathBuf::from(path)
}

fn default_user_data_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".local/share/spider/login-chrome-profile"))
}

fn resolve_output_path(output_dir: &Path, value: &str) -> PathBuf {
    let path = expand_tilde(value);
    if path.is_absolute() {
        path
    } else {
        output_dir.join(path)
    }
}

fn extract_image_src(value: &str) -> Option<String> {
    let value = value.trim();

    if let Some(rest) = value.strip_prefix('[') {
        let end = rest.find(']')?;
        let src = rest[..end].trim();
        if src.is_empty() {
            None
        } else {
            Some(src.to_string())
        }
    } else if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn resolve_image_url(base_url: &str, raw: &str) -> Option<String> {
    if raw.starts_with("data:") || raw.starts_with("blob:") || raw.starts_with("javascript:") {
        return None;
    }

    if let Ok(url) = Url::parse(raw) {
        return Some(url.to_string());
    }

    Url::parse(base_url)
        .ok()
        .and_then(|base| base.join(raw).ok())
        .map(|url| url.to_string())
}

fn infer_image_extension(image_url: &str, content_type: Option<&str>) -> &'static str {
    if let Ok(url) = Url::parse(image_url) {
        if let Some(ext) = Path::new(url.path())
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
        {
            match ext.as_str() {
                "jpg" | "jpeg" => return "jpg",
                "png" => return "png",
                "gif" => return "gif",
                "webp" => return "webp",
                "svg" => return "svg",
                "bmp" => return "bmp",
                "avif" => return "avif",
                _ => {}
            }
        }
    }

    match content_type.unwrap_or_default() {
        ct if ct.contains("image/png") => "png",
        ct if ct.contains("image/gif") => "gif",
        ct if ct.contains("image/webp") => "webp",
        ct if ct.contains("image/svg") => "svg",
        ct if ct.contains("image/bmp") => "bmp",
        ct if ct.contains("image/avif") => "avif",
        _ => "jpg",
    }
}

fn image_request_headers(
    headers: &HeaderMap,
    cookie: Option<&str>,
    referer_url: &str,
) -> Result<HeaderMap> {
    let mut image_headers = HeaderMap::new();

    for key in [USER_AGENT, ACCEPT_LANGUAGE] {
        if let Some(value) = headers.get(&key).cloned() {
            image_headers.insert(key, value);
        }
    }

    image_headers.insert(
        REFERER,
        HeaderValue::from_str(referer_url)
            .map_err(|err| Error::new(ErrorKind::InvalidInput, format!("invalid referer: {err}")))?,
    );

    if let Some(cookie) = cookie {
        image_headers.insert(
            COOKIE_HEADER,
            HeaderValue::from_str(cookie).map_err(|err| {
                Error::new(ErrorKind::InvalidInput, format!("invalid COOKIE header: {err}"))
            })?,
        );
    }

    Ok(image_headers)
}

async fn download_images(
    extracted: &HashMap<String, Vec<String>>,
    base_url: &str,
    output_dir: &Path,
    headers: &HeaderMap,
    cookie: Option<&str>,
) -> Result<usize> {
    let image_refs = match extracted.get("images") {
        Some(values) => values,
        None => return Ok(0),
    };

    let mut urls: Vec<String> = Vec::new();
    for value in image_refs {
        if let Some(src) = extract_image_src(value) {
            if let Some(url) = resolve_image_url(base_url, &src) {
                if !urls.iter().any(|existing| existing == &url) {
                    urls.push(url);
                }
            }
        }
    }

    if urls.is_empty() {
        return Ok(0);
    }

    let images_dir = output_dir.join("images");
    fs::create_dir_all(&images_dir)?;

    let image_headers = image_request_headers(headers, cookie, base_url)?;
    let client = Client::builder()
        .default_headers(image_headers)
        .build()
        .map_err(Error::other)?;

    let mut downloaded = 0usize;

    for (index, image_url) in urls.iter().enumerate() {
        let response = match client.get(image_url).send().await {
            Ok(response) if response.status().is_success() => response,
            _ => continue,
        };

        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = match response.bytes().await {
            Ok(bytes) if !bytes.is_empty() => bytes,
            _ => continue,
        };

        let ext = infer_image_extension(image_url, content_type.as_deref());
        let filename = format!("{:03}.{}", index + 1, ext);
        let path = images_dir.join(filename);

        if fs::write(&path, &bytes).is_ok() {
            downloaded += 1;
        }
    }

    Ok(downloaded)
}

fn resolve_browser_binary() -> Result<PathBuf> {
    if let Ok(path) = env::var("CHROME_BIN") {
        return Ok(expand_tilde(&path));
    }

    let candidates = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
    ];

    if let Some(path_os) = env::var_os("PATH") {
        for dir in env::split_paths(&path_os) {
            for name in candidates {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }

    let absolute_candidates = [
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/snap/bin/chromium",
        "/usr/bin/microsoft-edge",
    ];

    for candidate in absolute_candidates {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }

    Err(Error::new(
        ErrorKind::NotFound,
        "failed to find Chrome/Chromium binary; set CHROME_BIN explicitly",
    ))
}

fn local_debugging_port_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

fn wait_for_local_browser(port: u16, timeout: Duration) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let started = std::time::Instant::now();

    while started.elapsed() < timeout {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }

    Err(Error::new(
        ErrorKind::TimedOut,
        format!("timed out waiting for local browser debug port {port}"),
    ))
}

fn local_connection_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/json/version")
}

fn parse_extra_args() -> Vec<String> {
    env::var("CHROME_EXTRA_ARGS")
        .ok()
        .map(|value| value.split_whitespace().map(ToOwned::to_owned).collect())
        .unwrap_or_default()
}

fn apply_proxy_environment(command: &mut Command) {
    if env_truthy("CHROME_INHERIT_PROXY") {
        return;
    }

    for key in [
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
    ] {
        command.env_remove(key);
    }
}

fn launch_local_browser(
    browser_binary: &Path,
    user_data_dir: &Path,
    profile_dir: &str,
    port: u16,
    start_url: &str,
    headless: bool,
    extra_args: &[String],
) -> Result<Child> {
    fs::create_dir_all(user_data_dir)?;

    let mut command = Command::new(browser_binary);
    apply_proxy_environment(&mut command);
    command
        .arg(format!("--remote-debugging-port={port}"))
        .arg("--remote-debugging-address=127.0.0.1")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        .arg(format!("--profile-directory={profile_dir}"));

    if let Ok(proxy) = env::var("CHROME_PROXY") {
        if !proxy.trim().is_empty() {
            command.arg(format!("--proxy-server={}", proxy.trim()));
        }
    } else {
        command.arg("--no-proxy-server");
    }

    if headless {
        command.arg("--headless=new");
    }

    if !extra_args.is_empty() {
        command.args(extra_args);
    }

    command.arg(start_url).stdin(Stdio::null());

    command.spawn()
}

fn prepare_local_chrome(
    user_data_dir: PathBuf,
    profile_dir: String,
    port: u16,
    start_url: String,
    headless: bool,
    extra_args: &[String],
) -> Result<(String, Option<Child>, PathBuf, String)> {
    if local_debugging_port_open(port) {
        return Ok((
            local_connection_url(port),
            None,
            user_data_dir,
            profile_dir,
        ));
    }

    let browser_binary = resolve_browser_binary()?;
    let child = launch_local_browser(
        &browser_binary,
        &user_data_dir,
        &profile_dir,
        port,
        &start_url,
        headless,
        extra_args,
    )?;

    wait_for_local_browser(port, Duration::from_secs(20))?;

    Ok((
        local_connection_url(port),
        Some(child),
        user_data_dir,
        profile_dir,
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    let env = Env::default()
        .filter_or("RUST_LOG", "info")
        .write_style_or("RUST_LOG_STYLE", "always");
    env_logger::init_from_env(env);

    let cookie = env::var("COOKIE").ok();
    let prepare_profile = env_truthy("PREPARE_PROFILE");
    let chrome_headless = env_truthy("CHROME_HEADLESS");
    let chrome_profile_dir =
        env::var("CHROME_PROFILE_DIR").unwrap_or_else(|_| DEFAULT_PROFILE_DIR.to_string());
    let chrome_debugging_port = env::var("CHROME_DEBUGGING_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_DEBUGGING_PORT);
    let chrome_start_url =
        env::var("CHROME_START_URL").unwrap_or_else(|_| DEFAULT_START_URL.to_string());
    let chrome_extra_args = parse_extra_args();
    let mut chrome_connection_url = env::var("CHROME_CONNECTION_URL").ok();
    let target = env::args()
        .nth(1)
        .or_else(|| env::var("TARGET_URL").ok())
        .unwrap_or_else(|| DEFAULT_TARGET_URL.to_string());
    let output_dir = env::var("OUTPUT_DIR").unwrap_or_else(|_| DEFAULT_OUTPUT_DIR.to_string());
    let output_dir = expand_tilde(&output_dir);
    fs::create_dir_all(&output_dir)?;
    let output_html = env::var("OUTPUT_HTML").unwrap_or_else(|_| "page.html".to_string());
    let output_json =
        env::var("OUTPUT_JSON").unwrap_or_else(|_| "page_extracted.json".to_string());
    let output_html_path = resolve_output_path(&output_dir, &output_html);
    let output_json_path = resolve_output_path(&output_dir, &output_json);
    let wait_selector =
        env::var("WAIT_FOR_SELECTOR").unwrap_or_else(|_| DEFAULT_WAIT_SELECTOR.to_string());
    let download_images_enabled = !matches!(
        env::var("DOWNLOAD_IMAGES").ok().as_deref(),
        Some("0" | "false" | "False" | "FALSE")
    );

    let should_use_local_profile =
        chrome_connection_url.is_none() && (prepare_profile || cookie.is_none());

    let mut local_profile_summary: Option<(PathBuf, String)> = None;
    let mut _launched_browser: Option<Child> = None;

    if should_use_local_profile {
        let user_data_dir = env::var("CHROME_USER_DATA_DIR")
            .ok()
            .map(|value| expand_tilde(&value))
            .or_else(default_user_data_dir)
            .ok_or_else(|| Error::other("failed to resolve CHROME_USER_DATA_DIR"))?;

        let (connection_url, child, user_data_dir, profile_dir) = prepare_local_chrome(
            user_data_dir,
            chrome_profile_dir.clone(),
            chrome_debugging_port,
            chrome_start_url.clone(),
            chrome_headless,
            &chrome_extra_args,
        )?;

        chrome_connection_url = Some(connection_url);
        local_profile_summary = Some((user_data_dir, profile_dir));
        _launched_browser = child;
    }

    if prepare_profile {
        let (user_data_dir, profile_dir) = local_profile_summary.ok_or_else(|| {
            Error::other("PREPARE_PROFILE requires local browser profile preparation")
        })?;

        println!("Prepared local browser profile for authenticated browsing.");
        println!("User data dir: {}", user_data_dir.display());
        println!("Profile dir: {}", profile_dir);
        println!(
            "CDP endpoint: {}",
            chrome_connection_url
                .as_deref()
                .unwrap_or("http://127.0.0.1:9222/json/version")
        );
        println!(
            "Open the browser window, log in to the target site, then rerun without PREPARE_PROFILE=1."
        );
        println!("If you do not see a window, unset CHROME_HEADLESS or set it to 0.");
        return Ok(());
    }

    let mut headers = HeaderMap::new();
    let user_agent = env::var("USER_AGENT").unwrap_or_else(|_| {
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36".to_string()
    });
    let accept_language = env::var("ACCEPT_LANGUAGE_HEADER")
        .unwrap_or_else(|_| "zh-CN,zh;q=0.9,en;q=0.8".to_string());
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(&user_agent)
            .map_err(|err| Error::new(ErrorKind::InvalidInput, format!("invalid USER_AGENT: {err}")))?,
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_str(&accept_language).map_err(|err| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("invalid ACCEPT_LANGUAGE_HEADER: {err}"),
            )
        })?,
    );
    if let Ok(referer) = env::var("REFERER_URL") {
        headers.insert(
            REFERER,
            HeaderValue::from_str(&referer).map_err(|err| {
                Error::new(ErrorKind::InvalidInput, format!("invalid REFERER_URL: {err}"))
            })?,
        );
    }
    let headers_for_download = headers.clone();

    let mut website: Website = Website::new(&target);
    website
        .with_limit(1)
        .with_headers(Some(headers))
        .with_wait_for_idle_network0(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(5)))))
        .with_wait_for_selector(Some(WaitForSelector::new(
            Some(Duration::from_secs(20)),
            wait_selector.clone().into(),
        )))
        .with_wait_for_idle_dom(Some(WaitForSelector::new(
            Some(Duration::from_millis(800)),
            wait_selector.into(),
        )))
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true);

    if let Some(cookie) = cookie.as_deref() {
        website.with_cookies(cookie);
    }

    if let Some(ref chrome_connection_url) = chrome_connection_url {
        website.with_chrome_connection(Some(chrome_connection_url.clone()));
    }

    let mut website = website
        .build()
        .map_err(|_| Error::other("failed to build website config"))?;

    let mut rx = website
        .subscribe(16)
        .ok_or_else(|| Error::other("failed to subscribe to page stream"))?;

    let collector = tokio::spawn(async move {
        let mut pages: Vec<Page> = Vec::new();

        while let Ok(page) = rx.recv().await {
            pages.push(page);
        }

        pages
    });

    website.crawl().await;
    website.unsubscribe();

    let pages = collector
        .await
        .map_err(|err| Error::other(format!("collector task failed: {err}")))?;

    let page = pages
        .first()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "no page was captured"))?;

    let html = page.get_html();
    std::fs::write(&output_html_path, &html)?;

    let mut extracted =
        spider_utils::css_query_select_map_streamed(&html, &extraction_selectors()).await;
    post_process_extracted(&mut extracted);
    std::fs::write(
        &output_json_path,
        serde_json::to_vec_pretty(&extracted).map_err(json_error)?,
    )?;

    let downloaded_images = if download_images_enabled {
        download_images(
            &extracted,
            page.get_url_final(),
            &output_dir,
            &headers_for_download,
            cookie.as_deref(),
        )
        .await?
    } else {
        0
    };

    println!("Captured URL: {}", page.get_url());
    println!("Final URL: {}", page.get_url_final());
    println!("HTML bytes: {}", page.get_html_bytes_u8().len());
    if let Some((user_data_dir, profile_dir)) = local_profile_summary {
        println!(
            "Local profile reused: {} [{}]",
            user_data_dir.display(),
            profile_dir
        );
    } else if let Some(chrome_connection_url) = chrome_connection_url.as_deref() {
        println!("Chrome connection reused: {}", chrome_connection_url);
    } else if cookie.is_some() {
        println!("Authentication mode: cookie injection");
    }
    println!("Saved artifacts to dir: {}", output_dir.display());
    println!("Saved HTML to: {}", output_html_path.display());
    println!("Saved extracted content to: {}", output_json_path.display());
    if download_images_enabled {
        println!("Downloaded images: {}", downloaded_images);
        println!("Images dir: {}", output_dir.join("images").display());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&extracted).map_err(json_error)?
    );

    Ok(())
}
