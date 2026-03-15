use crate::options::{AuthenticatedPageArgs, Cli};
use spider::client::header::{
    HeaderMap, HeaderValue, ACCEPT_LANGUAGE, CONTENT_TYPE, COOKIE as COOKIE_HEADER, REFERER,
    USER_AGENT,
};
use spider::configuration::{WaitForDelay, WaitForIdleNetwork, WaitForSelector};
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::hashbrown::{HashMap, HashSet};
use spider::page::Page;
use spider::reqwest::{Client, Proxy, Url};
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
const DEFAULT_WAIT_SELECTOR: &str = "body";

fn split_selectors(value: &str) -> HashSet<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn extraction_selectors(args: &AuthenticatedPageArgs) -> spider_utils::DocumentSelectors<String> {
    let title = args
        .title_selectors
        .clone()
        .unwrap_or_else(|| "title,h1,h2".to_string());
    let content = args.content_selectors.clone().unwrap_or_else(|| {
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

    let mut selector_map: HashMap<String, HashSet<String>> = HashMap::new();
    selector_map.insert("title".to_string(), split_selectors(&title));
    selector_map.insert("content".to_string(), split_selectors(&content));
    selector_map.insert("images".to_string(), split_selectors(&args.image_selectors));

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

fn resolve_browser_binary(args: &AuthenticatedPageArgs) -> Result<PathBuf> {
    if let Some(path) = args.chrome_bin.as_deref() {
        return Ok(expand_tilde(path));
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
        "failed to find Chrome/Chromium binary; set --chrome-bin explicitly",
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

fn parse_extra_args(args: &AuthenticatedPageArgs) -> Vec<String> {
    args.chrome_extra_args
        .as_deref()
        .map(|value| value.split_whitespace().map(ToOwned::to_owned).collect())
        .unwrap_or_default()
}

fn launch_local_browser(
    browser_binary: &Path,
    user_data_dir: &Path,
    profile_dir: &str,
    port: u16,
    start_url: &str,
    headless: bool,
    extra_args: &[String],
    proxy: Option<&str>,
    inherit_proxy: bool,
) -> Result<Child> {
    fs::create_dir_all(user_data_dir)?;

    let mut command = Command::new(browser_binary);
    if !inherit_proxy {
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

    command
        .arg(format!("--remote-debugging-port={port}"))
        .arg("--remote-debugging-address=127.0.0.1")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        .arg(format!("--profile-directory={profile_dir}"));

    if let Some(proxy) = proxy {
        if !proxy.trim().is_empty() {
            command.arg(format!("--proxy-server={}", proxy.trim()));
        } else {
            command.arg("--no-proxy-server");
        }
    } else if !inherit_proxy {
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
    args: &AuthenticatedPageArgs,
    user_data_dir: PathBuf,
    start_url: &str,
) -> Result<(String, Option<Child>, PathBuf, String)> {
    if local_debugging_port_open(args.chrome_debugging_port) {
        return Ok((
            local_connection_url(args.chrome_debugging_port),
            None,
            user_data_dir,
            args.chrome_profile_dir.clone(),
        ));
    }

    let browser_binary = resolve_browser_binary(args)?;
    let extra_args = parse_extra_args(args);
    let child = launch_local_browser(
        &browser_binary,
        &user_data_dir,
        &args.chrome_profile_dir,
        args.chrome_debugging_port,
        start_url,
        args.chrome_headless,
        &extra_args,
        args.chrome_proxy.as_deref(),
        args.chrome_inherit_proxy,
    )?;

    wait_for_local_browser(args.chrome_debugging_port, Duration::from_secs(20))?;

    Ok((
        local_connection_url(args.chrome_debugging_port),
        Some(child),
        user_data_dir,
        args.chrome_profile_dir.clone(),
    ))
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
                Error::new(ErrorKind::InvalidInput, format!("invalid cookie header: {err}"))
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
    proxy: Option<&str>,
    inherit_proxy: bool,
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
    let mut client_builder = Client::builder().default_headers(image_headers);
    if let Some(proxy) = proxy {
        if !proxy.trim().is_empty() {
            client_builder = client_builder.proxy(Proxy::all(proxy).map_err(Error::other)?);
        }
    } else if !inherit_proxy {
        client_builder = client_builder.no_proxy();
    }

    let client = client_builder.build().map_err(Error::other)?;
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
        let path = images_dir.join(format!("{:03}.{}", index + 1, ext));
        if fs::write(&path, &bytes).is_ok() {
            downloaded += 1;
        }
    }

    Ok(downloaded)
}

fn build_headers(cli: &Cli, args: &AuthenticatedPageArgs) -> Result<(HeaderMap, HeaderMap)> {
    let mut headers = HeaderMap::new();
    let user_agent = args
        .user_agent
        .clone()
        .or_else(|| cli.agent.clone())
        .unwrap_or_else(|| {
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36".to_string()
        });
    let accept_language = args
        .accept_language_header
        .clone()
        .unwrap_or_else(|| "zh-CN,zh;q=0.9,en;q=0.8".to_string());

    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(&user_agent)
            .map_err(|err| Error::new(ErrorKind::InvalidInput, format!("invalid user agent: {err}")))?,
    );
    headers.insert(
        ACCEPT_LANGUAGE,
        HeaderValue::from_str(&accept_language).map_err(|err| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("invalid accept-language header: {err}"),
            )
        })?,
    );

    if let Some(referer) = args.referer_url.as_deref() {
        headers.insert(
            REFERER,
            HeaderValue::from_str(referer).map_err(|err| {
                Error::new(ErrorKind::InvalidInput, format!("invalid referer url: {err}"))
            })?,
        );
    }

    let headers_for_download = headers.clone();
    Ok((headers, headers_for_download))
}

fn target_url(cli: &Cli, args: &AuthenticatedPageArgs) -> Result<String> {
    if let Some(url) = args.url.clone().or_else(|| cli.url.clone()) {
        if url.starts_with("http") {
            Ok(url)
        } else {
            Ok(format!("https://{}", url))
        }
    } else if args.prepare_profile {
        Ok(DEFAULT_TARGET_URL.to_string())
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            "authenticated-page requires --url unless --prepare-profile is used",
        ))
    }
}

pub async fn run_authenticated_page(cli: &Cli, args: &AuthenticatedPageArgs) -> Result<()> {
    #[cfg(not(feature = "chrome"))]
    {
        let _ = cli;
        let _ = args;
        return Err(Error::other(
            "authenticated-page requires spider_cli to be built with the `chrome` feature",
        ));
    }

    #[cfg(feature = "chrome")]
    {
        let cookie = args.cookie.clone();
        let output_dir = expand_tilde(&args.output_dir);
        fs::create_dir_all(&output_dir)?;
        let output_html_path = resolve_output_path(&output_dir, &args.output_html);
        let output_json_path = resolve_output_path(&output_dir, &args.output_json);
        let download_images_enabled = !args.no_download_images;

        let target = target_url(cli, args)?;
        let chrome_start_url = args
            .chrome_start_url
            .clone()
            .unwrap_or_else(|| target.clone());

        let mut chrome_connection_url = args.chrome_connection_url.clone();
        let should_use_local_profile = chrome_connection_url.is_none() && (args.prepare_profile || cookie.is_none());

        let mut local_profile_summary: Option<(PathBuf, String)> = None;
        let mut _launched_browser: Option<Child> = None;

        if should_use_local_profile {
            let user_data_dir = args
                .chrome_user_data_dir
                .as_deref()
                .map(expand_tilde)
                .or_else(default_user_data_dir)
                .ok_or_else(|| Error::other("failed to resolve chrome user data dir"))?;

            let (connection_url, child, user_data_dir, profile_dir) =
                prepare_local_chrome(args, user_data_dir, &chrome_start_url)?;

            chrome_connection_url = Some(connection_url);
            local_profile_summary = Some((user_data_dir, profile_dir));
            _launched_browser = child;
        }

        if args.prepare_profile {
            let (user_data_dir, profile_dir) = local_profile_summary.ok_or_else(|| {
                Error::other("prepare-profile requires a local browser profile")
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
                "Open the browser window, log in to the target site, then rerun without --prepare-profile."
            );
            println!("If you do not see a window, unset --chrome-headless.");
            return Ok(());
        }

        let (headers, headers_for_download) = build_headers(cli, args)?;

        let mut website: Website = Website::new(&target);
        website
            .with_limit(1)
            .with_headers(Some(headers))
            .with_chrome_intercept(RequestInterceptConfiguration::new(true))
            .with_stealth(true);

        if let Some(delay) = cli.delay {
            website.with_delay(delay);
        }
        if let Some(wait_for_idle_network) = cli.wait_for_idle_network {
            website.with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(
                Duration::from_millis(wait_for_idle_network),
            ))));
        }
        if let Some(wait_for_idle_network0) = cli.wait_for_idle_network0 {
            website.with_wait_for_idle_network0(Some(WaitForIdleNetwork::new(Some(
                Duration::from_millis(wait_for_idle_network0),
            ))));
        } else {
            website.with_wait_for_idle_network0(Some(WaitForIdleNetwork::new(Some(
                Duration::from_secs(5),
            ))));
        }
        if let Some(wait_for_almost_idle_network0) = cli.wait_for_almost_idle_network0 {
            website.with_wait_for_almost_idle_network0(Some(WaitForIdleNetwork::new(Some(
                Duration::from_millis(wait_for_almost_idle_network0),
            ))));
        }
        if let Some(selector) = cli.wait_for_idle_dom.clone() {
            website.with_wait_for_idle_dom(Some(WaitForSelector::new(
                Some(Duration::from_secs(30)),
                selector,
            )));
        } else {
            let selector = cli
                .wait_for_selector
                .clone()
                .unwrap_or_else(|| DEFAULT_WAIT_SELECTOR.to_string());
            website.with_wait_for_idle_dom(Some(WaitForSelector::new(
                Some(Duration::from_millis(800)),
                selector,
            )));
        }
        if let Some(selector) = cli.wait_for_selector.clone() {
            website.with_wait_for_selector(Some(WaitForSelector::new(
                Some(Duration::from_secs(60)),
                selector,
            )));
        } else {
            website.with_wait_for_selector(Some(WaitForSelector::new(
                Some(Duration::from_secs(20)),
                DEFAULT_WAIT_SELECTOR.into(),
            )));
        }
        if let Some(wait_for_delay) = cli.wait_for_delay {
            website.with_wait_for_delay(Some(WaitForDelay::new(Some(Duration::from_millis(
                wait_for_delay,
            )))));
        }
        if let Some(limit) = cli.limit {
            website.with_limit(limit);
        }
        if let Some(depth) = cli.depth {
            website.with_depth(depth);
        }
        if let Some(proxy_url) = cli.proxy_url.clone() {
            if !proxy_url.is_empty() {
                website.with_proxies(Some(vec![proxy_url]));
            }
        }
        if let Some(cookie) = cookie.as_deref() {
            website.with_cookies(cookie);
        }
        if let Some(chrome_connection_url) = chrome_connection_url.clone() {
            website.with_chrome_connection(Some(chrome_connection_url));
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
        fs::write(&output_html_path, &html)?;

        let mut extracted =
            spider_utils::css_query_select_map_streamed(&html, &extraction_selectors(args)).await;
        post_process_extracted(&mut extracted);
        fs::write(
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
                args.chrome_proxy.as_deref().or(cli.proxy_url.as_deref()),
                args.chrome_inherit_proxy,
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
}
