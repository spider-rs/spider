use crate::tokio_stream::StreamExt;
use chromiumoxide::{Browser, BrowserConfig};
use tokio::task;

/// get chrome configuration
#[cfg(not(feature = "chrome_headed"))]
pub fn get_browser_config() -> Result<BrowserConfig, String> {
    BrowserConfig::builder().build()
}

/// get chrome configuration headful
#[cfg(feature = "chrome_headed")]
pub fn get_browser_config() -> Result<BrowserConfig, String> {
    BrowserConfig::builder().with_head().build()
}

/// launch a chromium browser and wait until the instance is up blocking the thread.
pub async fn launch_browser() -> (Browser, tokio::task::JoinHandle<()>) {
    let (browser, mut handler) = Browser::launch(get_browser_config().unwrap())
        .await
        .unwrap();

    // spawn a new task that continuously polls the handler
    let handle = task::spawn(async move {
        while let Some(h) = handler.next().await {
            if h.is_err() {
                break;
            }
        }
    });

    (browser, handle)
}
