use log::{info, log_enabled, Level};
use reqwest::Client;

use std::sync::Arc;
use tokio::sync::watch;
use tokio::sync::watch::Receiver;
use tokio::sync::watch::Sender;
use tokio::sync::Mutex;

#[cfg(not(feature = "fs"))]
/// Perform a network request to a resource extracting all content as text streaming.
pub async fn fetch_page_html(url: &str, client: &Client) -> Option<String> {
    use tokio_stream::StreamExt;

    match client.get(url).send().await {
        Ok(res) if res.status().is_success() => {
            let mut stream = res.bytes_stream();
            let mut data: String = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => data += &String::from_utf8_lossy(&text),
                    _ => (),
                }
            }

            Some(data)
        }
        Ok(_) => None,
        Err(_) => {
            log("- error parsing html text {}", &url);
            None
        }
    }
}

/// Perform a network request to a resource extracting all content as text.
#[cfg(feature = "decentralized")]
pub async fn fetch_page(url: &str, client: &Client) -> Option<bytes::Bytes> {
    match client.get(url).send().await {
        Ok(res) if res.status().is_success() => match res.bytes().await {
            Ok(text) => Some(text),
            Err(_) => {
                log("- error fetching {}", &url);
                None
            }
        },
        Ok(_) => None,
        Err(_) => {
            log("- error parsing html bytes {}", &url);
            None
        }
    }
}

/// Perform a network request to a resource extracting all content as text streaming.
#[cfg(feature = "fs")]
pub async fn fetch_page_html(url: &str, client: &Client) -> Option<String> {
    use crate::tokio::io::AsyncReadExt;
    use crate::tokio::io::AsyncWriteExt;
    use percent_encoding::utf8_percent_encode;
    use percent_encoding::NON_ALPHANUMERIC;
    use std::time::SystemTime;
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

    match client.get(url).send().await {
        Ok(res) if res.status().is_success() => {
            let mut stream = res.bytes_stream();
            let mut data = Box::new(String::new());
            let mut file: Option<tokio::fs::File> = None;
            let mut file_path = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => {
                        let wrote_disk = file.is_some();

                        // perform operations entire in memory to build resource
                        if !wrote_disk && data.capacity() < 8192 {
                            *data += &String::from_utf8_lossy(&text);
                        } else {
                            if !wrote_disk {
                                file_path = string_concat!(
                                    TMP_DIR,
                                    &utf8_percent_encode(url, NON_ALPHANUMERIC).to_string()
                                );
                                match tokio::fs::File::create(&file_path).await {
                                    Ok(f) => {
                                        let file = file.insert(f);

                                        *data += &String::from_utf8_lossy(&text);

                                        match file.write_all(data.as_bytes()).await {
                                            Ok(_) => {
                                                data.clear();
                                                data.shrink_to(0);
                                            }
                                            _ => (),
                                        };
                                    }
                                    _ => *data += &String::from_utf8_lossy(&text),
                                };
                            } else {
                                match &file.as_mut().unwrap().write_all(&text).await {
                                    Ok(_) => (),
                                    _ => *data += &String::from_utf8_lossy(&text),
                                };
                            }
                        }
                    }
                    _ => (),
                }
            }

            // get data from disk
            Some(if file.is_some() {
                let mut buffer = String::new();
                let data: String = match tokio::fs::File::open(&file_path).await {
                    Ok(mut b) => match b.read_to_string(&mut buffer).await {
                        Ok(_) => buffer,
                        _ => *data,
                    },
                    _ => *data,
                };

                match tokio::fs::remove_file(file_path).await {
                    _ => (),
                };

                data
            } else {
                *data
            })
        }
        Ok(_) => None,
        Err(_) => {
            log("- error parsing html text {}", &url);
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

lazy_static! {
    /// control handle for crawls
    pub static ref CONTROLLER: Arc<Mutex<(Sender<(String, Handler)>, Receiver<(String, Handler)>)>> = Arc::new(Mutex::new(watch::channel(("handles".to_string(), Handler::Start))));
}

/// pause a target website running crawl
pub async fn pause(domain: &str) {
    let s = CONTROLLER.clone();

    s.lock()
        .await
        .0
        .send((domain.to_string(), Handler::Pause))
        .unwrap();
}

/// resume a target website crawl
pub async fn resume(domain: &str) {
    let s = CONTROLLER.clone();

    s.lock()
        .await
        .0
        .send((domain.to_string(), Handler::Resume))
        .unwrap();
}

/// shutdown a target website crawl
pub async fn shutdown(domain: &str) {
    let s = CONTROLLER.clone();

    s.lock()
        .await
        .0
        .send((domain.to_string(), Handler::Shutdown))
        .unwrap();
}
