// This example demonstrates how to capture console logs.

use std::time::Duration;

use chromiumoxide::{cdp::js_protocol::runtime::EventConsoleApiCalled, BrowserConfig};
use futures::StreamExt;

const TARGET: &str = "https://www.microsoft.com/";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let (browser, mut handler) =
        chromiumoxide::Browser::launch(BrowserConfig::builder().with_head().build().unwrap())
            .await
            .expect("failed to launch browser");

    let handle = tokio::task::spawn(async move {
        while let Some(event) = handler.next().await {
            tracing::debug!(event = ?event);
        }
    });

    let page = browser
        .new_page("about:blank")
        .await
        .expect("failed to create page");

    let mut console_events = page
        .event_listener::<EventConsoleApiCalled>()
        .await
        .expect("Failed to add event listener");
    let logs_handle = tokio::spawn(async move {
        while let Some(event) = console_events.next().await {
            println!(
                "{}",
                serde_json::to_string_pretty(&*event).expect("Failed to serialize event")
            );
        }
    });

    let _ = page.goto(TARGET).await.expect("failed to navigate");

    tokio::time::sleep(Duration::from_secs(3)).await;

    browser.close().await.expect("Failed to close browser");
    logs_handle.await.expect("Failed to wait for logs handle");
    handle.await.expect("Failed to await handle");

    // Give browser time to finish closing.
    tokio::time::sleep(Duration::from_secs(1)).await;
}
