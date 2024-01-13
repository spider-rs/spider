use crate::Client;
use log::{info, log_enabled, Level};
use reqwest::{Error, Response, StatusCode};

/// The response of a web page.
#[derive(Debug, Default)]
pub struct PageResponse {
    /// The page response resource.
    pub content: Option<bytes::Bytes>,
    /// The status code of the request.
    pub status_code: StatusCode,
    /// The final url destination after any redirects.
    pub final_url: Option<String>,
    /// The message of the response error if any.
    pub error_for_status: Option<Result<Response, Error>>,
}

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html_chrome_base(
    target_url: &str,
    page: &chromiumoxide::Page,
    content: bool,
    wait: bool,
) -> Result<PageResponse, chromiumoxide::error::CdpError> {
    let page = page.activate().await?;

    let page = if content {
        page.set_content(target_url).await?
    } else {
        page.goto(target_url).await?
    };

    let final_url = if wait {
        match page.wait_for_navigation_response().await {
            Ok(u) => get_last_redirect(&target_url, &u),
            _ => None,
        }
    } else {
        None
    };

    let res = page.content_bytes().await;
    let ok = res.is_ok();

    Ok(PageResponse {
        content: if ok {
            Some(res.unwrap_or_default())
        } else {
            None
        },
        // todo: get the cdp error to status code.
        status_code: if ok {
            StatusCode::OK
        } else {
            Default::default()
        },
        final_url,
        ..Default::default()
    })
}

#[cfg(all(
    not(feature = "fs"),
    feature = "chrome",
    not(feature = "chrome_screenshot")
))]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
) -> PageResponse {
    match fetch_page_html_chrome_base(&target_url, &page, false, true).await {
        Ok(page) => page,
        _ => fetch_page_html_raw(&target_url, &client).await,
    }
}

#[cfg(all(not(feature = "fs"), feature = "chrome", feature = "chrome_screenshot"))]
/// Perform a network request to a resource extracting all content as text streaming via chrome storing screenshots for each page.
pub async fn fetch_page_html(
    target_url: &str,
    client: &Client,
    page: &chromiumoxide::Page,
) -> PageResponse {
    let page = page.activate().await;

    match page {
        Ok(page) => {
            match page.goto(target_url).await {
                Ok(page) => {
                    let p = page.wait_for_navigation_response().await;
                    let res = page.content_bytes().await;
                    let ok = res.is_ok();

                    let output_path = string_concat!(
                        std::env::var("SCREENSHOT_DIRECTORY")
                            .unwrap_or_else(|_| "./storage/".to_string()),
                        &percent_encoding::percent_encode(
                            target_url.as_bytes(),
                            percent_encoding::NON_ALPHANUMERIC
                        )
                        .to_string(),
                        ".png"
                    );

                    let output_path = std::path::Path::new(&output_path);

                    match output_path.parent() {
                        Some(p) => {
                            let _ = tokio::fs::create_dir_all(&p).await;
                        }
                        _ => (),
                    }

                    match page.save_screenshot(
                        chromiumoxide::page::ScreenshotParams::builder()
                            .format(chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::Png)
                            .full_page(match std::env::var("SCREENSHOT_FULL_PAGE") {
                                Ok(t) => t == "true",
                                _ => true
                            })
                            .omit_background(match std::env::var("SCREENSHOT_OMIT_BACKGROUND") {
                                Ok(t) => t == "true",
                                _ => true
                            })
                            .build(),
                           &output_path,
                    )
                    .await {
                        Ok(_) => log::debug!("saved screenshot: {:?}", output_path),
                        Err(e) => log::error!("failed to save screenshot: {:?} - {:?}", e, output_path)
                    };

                    PageResponse {
                        content: if ok {
                            Some(res.unwrap_or_default())
                        } else {
                            None
                        },
                        // todo: get the cdp error to status code.
                        status_code: if ok {
                            StatusCode::OK
                        } else {
                            Default::default()
                        },
                        final_url: match p {
                            Ok(u) => get_last_redirect(&target_url, &u),
                            _ => None,
                        },
                        ..Default::default()
                    }
                }
                _ => fetch_page_html_raw(&target_url, &client).await,
            }
        }
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
    use tokio_stream::StreamExt;

    match client.get(target_url).send().await {
        Ok(res) if res.status().is_success() => {
            let u = res.url().as_str();

            let rd = if target_url != u {
                Some(u.into())
            } else {
                None
            };
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
                content: Some(data.into()),
                final_url: rd,
                status_code,
                ..Default::default()
            }
        }
        Ok(res) => PageResponse {
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
    use tokio_stream::StreamExt;

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
                final_url: rd,
                ..Default::default()
            }
        }
        Ok(_) => Default::default(),
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
) -> PageResponse {
    match &page {
        page => match fetch_page_html_chrome_base(&target_url, &page, false, true).await {
            Ok(page) => page,
            _ => {
                log(
                    "- error parsing html text defaulting to raw http request {}",
                    &target_url,
                );

                use crate::bytes::BufMut;
                use bytes::BytesMut;
                use tokio_stream::StreamExt;

                let content = match client.get(target_url).send().await {
                    Ok(res) if res.status().is_success() => {
                        let mut stream = res.bytes_stream();
                        let mut data: BytesMut = BytesMut::new();

                        while let Some(item) = stream.next().await {
                            match item {
                                Ok(text) => data.put(text),
                                _ => (),
                            }
                        }

                        Some(data.into())
                    }
                    Ok(_) => None,
                    Err(_) => {
                        log("- error parsing html text {}", &target_url);
                        None
                    }
                };

                PageResponse {
                    content: content,
                    ..Default::default()
                }
            }
        },
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
