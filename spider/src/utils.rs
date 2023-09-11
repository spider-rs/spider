#[cfg(feature = "chrome")]
use crate::website::CHROME;

use log::{info, log_enabled, Level};
use reqwest::Client;

#[cfg(feature = "chrome")]
/// Perform a network request to a resource extracting all content as text streaming via chrome.
pub async fn fetch_page_html(target_url: &str, client: &Client) -> Option<bytes::Bytes> {
    match &CHROME.0 {
        Some(browser) => match browser.new_page(target_url).await {
            Ok(page) => {
                let res = page.content().await;
                let content = res.unwrap_or_default().into();

                let _ = page.close().await;

                Some(content)
            }
            _ => Default::default(),
        },
        _ => {
            log(
                "- error parsing html text defaulting to raw http request {}",
                &target_url,
            );

            use crate::bytes::BufMut;
            use bytes::BytesMut;
            use tokio_stream::StreamExt;

            match client.get(target_url).send().await {
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
            }
        }
    }
}

#[cfg(all(not(feature = "fs"), not(feature = "chrome")))]
/// Perform a network request to a resource extracting all content as text streaming.
pub async fn fetch_page_html(target_url: &str, client: &Client) -> Option<bytes::Bytes> {
    use crate::bytes::BufMut;
    use bytes::BytesMut;
    use tokio_stream::StreamExt;

    match client.get(target_url).send().await {
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
    }
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
pub async fn fetch_page_html(target_url: &str, client: &Client) -> Option<bytes::Bytes> {
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

            // get data from disk
            Some(if file.is_some() {
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
            })
        }
        Ok(_) => None,
        Err(_) => {
            log("- error parsing html text {}", &target_url);
            None
        }
    }
}

/// log to console if configuration verbose.
pub fn log(message: &'static str, data: impl AsRef<str>) {
    if log_enabled!(Level::Info) {
        info!("{message} - {}", data.as_ref());
    }
}

#[cfg(feature = "control")]
/// determine action
#[derive(PartialEq, Debug)]
pub enum Handler {
    /// crawl start state
    Start,
    /// crawl pause state
    Pause,
    /// crawl resume
    Resume,
    /// crawl shutdown
    Shutdown,
}

#[cfg(feature = "control")]
lazy_static! {
    /// control handle for crawls
    pub static ref CONTROLLER: std::sync::Arc<tokio::sync::Mutex<(tokio::sync::watch::Sender<(String, Handler)>, tokio::sync::watch::Receiver<(String, Handler)>)>> = std::sync::Arc::new(tokio::sync::Mutex::new(tokio::sync::watch::channel(("handles".to_string(), Handler::Start))));
}

#[cfg(feature = "control")]
/// pause a target website running crawl
pub async fn pause(domain: &str) {
    let s = CONTROLLER.clone();

    match s.lock().await.0.send((domain.into(), Handler::Pause)) {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// resume a target website crawl
pub async fn resume(domain: &str) {
    let s = CONTROLLER.clone();

    match s.lock().await.0.send((domain.into(), Handler::Resume)) {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// shutdown a target website crawl
pub async fn shutdown(domain: &str) {
    let s = CONTROLLER.clone();

    match s.lock().await.0.send((domain.into(), Handler::Shutdown)) {
        _ => (),
    };
}

#[cfg(feature = "control")]
/// reset a target website crawl
pub async fn reset(domain: &str) {
    let s = CONTROLLER.clone();

    match s.lock().await.0.send((domain.into(), Handler::Start)) {
        _ => (),
    };
}
