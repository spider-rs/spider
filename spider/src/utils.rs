use log::{info, log_enabled, Level};
use reqwest::Client;
use reqwest::StatusCode;

use std::str;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::sync::watch::Receiver;
use tokio::sync::watch::Sender;
use tokio::sync::Mutex;

/// Perform a network request to a resource extracting all content as text streaming.
pub async fn fetch_page_html(url: &str, client: &Client) -> Option<String> {
    use tokio_stream::StreamExt;

    match client.get(url).send().await {
        Ok(res) if res.status() == StatusCode::OK => {
            let mut stream = res.bytes_stream();
            let mut data: String = String::new();

            while let Some(item) = stream.next().await {
                match item {
                    Ok(text) => {
                        // already valid utf8
                        data += unsafe { str::from_utf8_unchecked(&text) };
                    }
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
        Ok(res) if res.status() == StatusCode::OK => match res.bytes().await {
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
