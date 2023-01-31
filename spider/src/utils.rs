use log::{info, log_enabled, Level};
use reqwest::Client;
use reqwest::StatusCode;

use std::sync::Arc;
use tokio::sync::watch;
use tokio::sync::watch::Receiver;
use tokio::sync::watch::Sender;
use tokio::sync::Mutex;

/// Perform a network request to a resource extracting all content as text.
pub async fn fetch_page_html(url: &str, client: &Client) -> String {
    match client.get(url).send().await {
        Ok(res) if res.status() == StatusCode::OK => match res.text().await {
            Ok(text) => text,
            Err(_) => {
                log("- error fetching {}", url);

                String::new()
            }
        },
        Ok(_) => String::new(),
        Err(_) => {
            log("- error parsing html text {}", url);
            String::new()
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
