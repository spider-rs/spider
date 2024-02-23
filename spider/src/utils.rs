use crate::tokio_stream::StreamExt;
use crate::Client;
use log::{info, log_enabled, Level};
#[cfg(feature = "headers")]
use reqwest::header::HeaderMap;
use reqwest::{Error, Response, StatusCode};

/// The response of a web page.
#[derive(Debug, Default)]
pub struct PageResponse {
    /// The page response resource.
    pub content: Option<bytes::Bytes>,
    #[cfg(feature = "headers")]
    /// The headers of the response. (Always None if a webdriver protocol is used for fetching.).
    pub headers: Option<HeaderMap>,
    /// The status code of the request.
    pub status_code: StatusCode,
    /// The final url destination after any redirects.
    pub final_url: Option<String>,
    /// The message of the response error if any.
    pub error_for_status: Option<Result<Response, Error>>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page. The ScreenShotConfig bytes boolean needs to be set to true.
    pub screenshot_bytes: Option<Vec<u8>>,
}

/// wait for event with timeout
#[cfg(feature = "chrome")]
pub async fn wait_for_event<T>(page: &chromiumoxide::Page, timeout: Option<core::time::Duration>)
where
    T: chromiumoxide::cdp::IntoEventKind + Unpin,
{
    match page.event_listener::<T>().await {
        Ok(mut events) => {
            let wait_until = async {
                loop {
                    let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(500));
                    tokio::pin!(sleep);
                    tokio::select! {
                        _ = &mut sleep => (),
                        v = events.next() => {
                          if v.is_none () {
                              break;
                          }
                        }
                    }
                }
            };
            match timeout {
                Some(timeout) => if let Err(_) = tokio::time::timeout(timeout, wait_until).await {},
                _ => wait_until.await,
            }
        }
        _ => (),
    }
}

/// wait for a selector
#[cfg(feature = "chrome")]
pub async fn wait_for_selector(
    page: &chromiumoxide::Page,
    timeout: Option<core::time::Duration>,
    selector: &str,
) {
    let wait_until = async {
        loop {
            let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(50));
            tokio::pin!(sleep);
            tokio::select! {
                _ = &mut sleep => (),
                v = page.find_element(selector) => {
                    if !v.is_ok() {
                        break
                    }
                }
            }
        }
    };
    match timeout {
        Some(timeout) => if let Err(_) = tokio::time::timeout(timeout, wait_until).await {},
        _ => wait_until.await,
    }
}

/// Get the output path of a screenshot and create any parent folders if needed.
#[cfg(feature = "chrome")]
pub async fn create_output_path(
    base: &std::path::PathBuf,
    target_url: &str,
    format: &str,
) -> String {
    let out = string_concat!(
        &percent_encoding::percent_encode(
            target_url.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC
        )
        .to_string(),
        format
    );

    match base.join(&out).parent() {
        Some(p) => {
            let _ = tokio::fs::create_dir_all(&p).await;
        }
        _ => (),
    }

    out
}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_chrome_base(
    target_url: &str,
    page: &chromiumoxide::Page,
    content: bool,
    wait_for_navigation: bool,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
) -> Result<PageResponse, chromiumoxide::error::CdpError> {
    // used for smart mode re-rendering direct assigning html
    let page = if content {
        page.set_content(target_url).await?
    } else {
        page.goto(target_url).await?
    };

    // we do not need to wait for navigation if content is assigned. The method set_content already handles this.
    let final_url = if wait_for_navigation && !content {
        match page.wait_for_navigation_response().await {
            Ok(u) => get_last_redirect(&target_url, &u),
            _ => None,
        }
    } else {
        None
    };

    match wait_for {
        Some(wait_for) => {
            match wait_for.idle_network {
                Some(ref network_idle) => {
                    wait_for_event::<
                        chromiumoxide::cdp::browser_protocol::network::EventLoadingFinished,
                    >(page, network_idle.timeout)
                    .await;
                }
                _ => (),
            }

            match wait_for.selector {
                Some(ref await_for_selector) => {
                    wait_for_selector(
                        page,
                        await_for_selector.timeout,
                        &await_for_selector.selector,
                    )
                    .await;
                }
                _ => (),
            }

            match wait_for.delay {
                Some(ref wait_for_delay) => match wait_for_delay.timeout {
                    Some(timeout) => tokio::time::sleep(timeout).await,
                    _ => (),
                },
                _ => (),
            }
        }
        _ => (),
    }

    let page = page.activate().await?;
    let res: bytes::Bytes = page.content_bytes().await?;
    let ok = res.len() > 0;

    let mut page_response = PageResponse {
        content: if ok { Some(res) } else { None },
        // todo: get the cdp error to status code.
        status_code: if ok {
            StatusCode::OK
        } else {
            Default::default()
        },
        final_url,
        ..Default::default()
    };

    if cfg!(feature = "chrome_screenshot") || screenshot.is_some() {
        perform_screenshot(target_url, page, screenshot, &mut page_response).await;
    }

    if cfg!(not(feature = "chrome_store_page")) {
        page.execute(chromiumoxide::cdp::browser_protocol::page::CloseParams::default())
            .await?;
    }

    Ok(page_response)
}

/// Perform a screenshot shortcut.
#[cfg(feature = "chrome")]
pub async fn perform_screenshot(
    target_url: &str,
    page: &chromiumoxide::Page,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
    page_response: &mut PageResponse,
) {
    match screenshot {
        Some(ref ss) => {
            let output_format = string_concat!(
                ".",
                ss.params
                    .cdp_params
                    .format
                    .clone()
                    .unwrap_or_else(|| crate::configuration::CaptureScreenshotFormat::Png)
                    .to_string()
            );
            let ss_params = chromiumoxide::page::ScreenshotParams::from(ss.params.clone());

            if ss.save {
                let output_path = create_output_path(
                    &ss.output_dir.clone().unwrap_or_else(|| "./storage/".into()),
                    &target_url,
                    &output_format,
                )
                .await;

                match page.save_screenshot(ss_params, &output_path).await {
                    Ok(b) => {
                        log::debug!("saved screenshot: {:?}", output_path);
                        if ss.bytes {
                            page_response.screenshot_bytes = Some(b);
                        }
                    }
                    Err(e) => {
                        log::error!("failed to save screenshot: {:?} - {:?}", e, output_path)
                    }
                };
            } else {
                match page.screenshot(ss_params).await {
                    Ok(b) => {
                        log::debug!("took screenshot: {:?}", target_url);
                        if ss.bytes {
                            page_response.screenshot_bytes = Some(b);
                        }
                    }
                    Err(e) => {
                        log::error!("failed to take screenshot: {:?} - {:?}", e, target_url)
                    }
                };
            }
        }
        _ => {
            let output_path = create_output_path(
                &std::env::var("SCREENSHOT_DIRECTORY")
                    .unwrap_or_else(|_| "./storage/".to_string())
                    .into(),
                &target_url,
                &".png",
            )
            .await;

            match page
                .save_screenshot(
                    chromiumoxide::page::ScreenshotParams::builder()
                        .format(
                            chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png,
                        )
                        .full_page(match std::env::var("SCREENSHOT_FULL_PAGE") {
                            Ok(t) => t == "true",
                            _ => true,
                        })
                        .omit_background(match std::env::var("SCREENSHOT_OMIT_BACKGROUND") {
                            Ok(t) => t == "true",
                            _ => true,
                        })
                        .build(),
                    &output_path,
                )
                .await
            {
                Ok(_) => log::debug!("saved screenshot: {:?}", output_path),
                Err(e) => log::error!("failed to save screenshot: {:?} - {:?}", e, output_path),
            };
        }
    }
}

#[cfg(all(not(feature = "fs"), feature = "chrome"))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
) -> PageResponse {
    match fetch_page_html_chrome_base(&target_url, &page, false, true, wait_for, screenshot).await {
        Ok(page) => page,
        _ => fetch_page_html_raw(&target_url, &client).await,
    }
}

#[cfg(feature = "chrome")]
/// Check if url matches the last item in a redirect chain for chrome CDP
pub fn get_last_redirect(
    target_url: &str,
    u: &Option<std::sync::Arc<chromiumoxide::handler::http::HttpRequest>>,
) -> Option<String> {
    match u {
        Some(u) => match u.redirect_chain.last()? {
            r => match r.url.as_ref()? {
                u => {
                    if target_url != u {
                        Some(u.into())
                    } else {
                        None
                    }
                }
            },
        },
        _ => None,
    }
}

/// Perform a network request to a resource extracting all content streaming.
pub async fn fetch_page_html_raw(target_url: &str, client: &Client) -> PageResponse {
    use crate::bytes::BufMut;
    use bytes::BytesMut;

    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
            let u = res.url().as_str();

            let rd = if target_url != u {
                Some(u.into())
            } else {
                None
            };
            let status_code = res.status();
            #[cfg(feature = "headers")]
            let headers = res.headers().clone();
            let mut stream = res.bytes_stream();
            let mut data: BytesMut = BytesMut::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => data.put(text),
                    _ => (),
                }
            }

            PageResponse {
                #[cfg(feature = "headers")]
                headers: Some(headers),
                content: Some(data.into()),
                final_url: rd,
                status_code,
                ..Default::default()
            }
        }
        Ok(res) => PageResponse {
            #[cfg(feature = "headers")]
            headers: Some(res.headers().clone()),
            status_code: res.status(),
            ..Default::default()
        },
        Err(_) => {
            log("- error parsing html text {}", target_url);
            Default::default()
        }
    }
}

#[cfg(all(not(feature = "fs"), not(feature = "chrome")))]
/// Perform a network request to a resource extracting all content as text streaming.
pub async fn fetch_page_html(target_url: &str, client: &Client) -> PageResponse {
    fetch_page_html_raw(target_url, client).await
}

/// Perform a network request to a resource extracting all content as text.
#[cfg(feature = "decentralized")]
pub async fn fetch_page(target_url: &str, client: &Client) -> Option<bytes::Bytes> {
    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => match res.bytes().await {
            Ok(text) => Some(text),
            Err(_) => {
                log("- error fetching {}", &target_url);
                None
            }
        },
        Ok(_) => None,
        Err(_) => {
            log("- error parsing html bytes {}", &target_url);
            None
        }
    }
}

#[cfg(all(feature = "decentralized", feature = "headers"))]
/// Fetch a page with the headers returned.
pub enum FetchPageResult {
    /// Success extracting contents of the page
    Success(HeaderMap, Option<bytes::Bytes>),
    /// No success extracting content
    NoSuccess(HeaderMap),
    /// A network error occured.
    FetchError,
}

#[cfg(all(feature = "decentralized", feature = "headers"))]
/// Perform a network request to a resource with the response headers..
pub async fn fetch_page_and_headers(target_url: &str, client: &Client) -> FetchPageResult {
    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
            let headers = res.headers().clone();
            let b = match res.bytes().await {
                Ok(text) => Some(text),
                Err(_) => {
                    log("- error fetching {}", &target_url);
                    None
                }
            };
            FetchPageResult::Success(headers, b)
        }
        Ok(res) => FetchPageResult::NoSuccess(res.headers().clone()),
        Err(_) => {
            log("- error parsing html bytes {}", &target_url);
            FetchPageResult::FetchError
        }
    }
}

/// Perform a network request to a resource extracting all content as text streaming.
#[cfg(feature = "fs")]
pub async fn fetch_page_html(target_url: &str, client: &Client) -> PageResponse {
    use crate::bytes::BufMut;
    use crate::tokio::io::AsyncReadExt;
    use crate::tokio::io::AsyncWriteExt;
    use bytes::BytesMut;
    use percent_encoding::utf8_percent_encode;
    use percent_encoding::NON_ALPHANUMERIC;
    use std::time::SystemTime;
    use tendril::fmt::Slice;

    lazy_static! {
        static ref TMP_DIR: String = {
            use std::fs;
            let mut tmp = std::env::temp_dir();

            tmp.push("spider/");

            // make sure spider dir is created.
            match fs::create_dir_all(&tmp) {
                Ok(_) => {
                    let dir_name = tmp.display().to_string();

                    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                        Ok(dur) => {
                            string_concat!(dir_name, dur.as_secs().to_string())
                        }
                        _ => dir_name,
                    }
                }
                _ => "/tmp/".to_string()
            }
        };
    };

    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
            let u = res.url().as_str();

            let rd = if target_url != u {
                Some(u.into())
            } else {
                None
            };

            let status_code = res.status();
            #[cfg(feature = "headers")]
            let headers = res.headers().clone();
            let mut stream = res.bytes_stream();
            let mut data: BytesMut = BytesMut::new();
            let mut file: Option<tokio::fs::File> = None;
            let mut file_path = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => {
                        let wrote_disk = file.is_some();

                        // perform operations entire in memory to build resource
                        if !wrote_disk && data.capacity() < 8192 {
                            data.put(text);
                        } else {
                            if !wrote_disk {
                                file_path = string_concat!(
                                    TMP_DIR,
                                    &utf8_percent_encode(target_url, NON_ALPHANUMERIC).to_string()
                                );
                                match tokio::fs::File::create(&file_path).await {
                                    Ok(f) => {
                                        let file = file.insert(f);

                                        data.put(text);

                                        match file.write_all(data.as_bytes()).await {
                                            Ok(_) => {
                                                data.clear();
                                            }
                                            _ => (),
                                        };
                                    }
                                    _ => data.put(text),
                                };
                            } else {
                                match &file.as_mut().unwrap().write_all(&text).await {
                                    Ok(_) => (),
                                    _ => data.put(text),
                                };
                            }
                        }
                    }
                    _ => (),
                }
            }

            PageResponse {
                #[cfg(feature = "headers")]
                headers: Some(headers),
                content: Some(if file.is_some() {
                    let mut buffer = vec![];

                    match tokio::fs::File::open(&file_path).await {
                        Ok(mut b) => match b.read_to_end(&mut buffer).await {
                            _ => (),
                        },
                        _ => (),
                    };

                    match tokio::fs::remove_file(file_path).await {
                        _ => (),
                    };

                    buffer.into()
                } else {
                    data.into()
                }),
                status_code,
                final_url: rd,
                ..Default::default()
            }
        }
        Ok(res) => PageResponse {
            #[cfg(feature = "headers")]
            headers: Some(res.headers().clone()),
            status_code: res.status(),
            ..Default::default()
        },
        Err(_) => {
            log("- error parsing html text {}", &target_url);
            Default::default()
        }
    }
}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_chrome(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
    wait_for: &Option<crate::configuration::WaitFor>,
    screenshot: &Option<crate::configuration::ScreenShotConfig>,
) -> PageResponse {
    match &page {
        page => {
            match fetch_page_html_chrome_base(&target_url, &page, false, true, wait_for, screenshot)
                .await
            {
                Ok(page) => page,
                _ => {
                    log(
                        "- error parsing html text defaulting to raw http request {}",
                        &target_url,
                    );

                    use crate::bytes::BufMut;
                    use bytes::BytesMut;

                    match client.get(target_url).send().await {
                        Ok(res) if res.status().is_success() => {
                            #[cfg(feature = "headers")]
                            let headers = res.headers().clone();
                            let status_code = res.status();
                            let mut stream = res.bytes_stream();
                            let mut data: BytesMut = BytesMut::new();

                            while let Some(item) = stream.next().await {
                                match item {
                                    Ok(text) => data.put(text),
                                    _ => (),
                                }
                            }

                            PageResponse {
                                #[cfg(feature = "headers")]
                                headers: Some(headers),
                                content: Some(data.into()),
                                status_code,
                                ..Default::default()
                            }
                        }
                        Ok(res) => PageResponse {
                            #[cfg(feature = "headers")]
                            headers: Some(res.headers().clone()),
                            status_code: res.status(),
                            ..Default::default()
                        },
                        Err(_) => {
                            log("- error parsing html text {}", &target_url);
                            Default::default()
                        }
                    }
                }
            }
        }
    }
}

/// Log to console if configuration verbose.
pub fn log(message: &'static str, data: impl AsRef<str>) {
    if log_enabled!(Level::Info) {
        info!("{message} - {}", data.as_ref());
    }
}

#[cfg(feature = "control")]
/// determine action
#[derive(PartialEq, Debug)]
pub enum Handler {
    /// Crawl start state
    Start,
    /// Crawl pause state
    Pause,
    /// Crawl resume
    Resume,
    /// Crawl shutdown
    Shutdown,
}

#[cfg(feature = "control")]
lazy_static! {
    /// control handle for crawls
    pub static ref CONTROLLER: std::sync::Arc<tokio::sync::RwLock<(tokio::sync::watch::Sender<(String, Handler)>,
        tokio::sync::watch::Receiver<(String, Handler)>)>> =
            std::sync::Arc::new(tokio::sync::RwLock::new(tokio::sync::watch::channel(("handles".to_string(), Handler::Start))));
}

#[cfg(feature = "control")]
/// Pause a target website running crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn pause(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Pause))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Resume a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn resume(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Resume))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Shutdown a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn shutdown(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Shutdown))
    {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// Reset a target website crawl. The crawl_id is prepended directly to the domain and required if set. ex: d22323edsd-https://mydomain.com
pub async fn reset(target: &str) {
    match CONTROLLER
        .write()
        .await
        .0
        .send((target.into(), Handler::Start))
    {
        _ => (),
    };
}
