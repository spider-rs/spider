// This example is for checking the iframe workaround.
// a problem with the iframe workaround is that it will always fail to load the page
// and goto will cause a timeout.

use chromiumoxide::handler::HandlerConfig;
use futures::StreamExt;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let (browser, mut handler) = chromiumoxide::Browser::connect_with_config(
        "ws://127.0.0.1:9222/devtools/browser/191fdaef-494d-41b5-8b94-4abd04dff33c",
        HandlerConfig::default(),
    )
    .await
    .expect("failed to connect to browser");

    let _ = tokio::task::spawn(async move {
        while let Some(event) = handler.next().await {
            tracing::debug!(event = ?event);
        }
    });

    let page = browser
        .new_page("about:blank")
        .await
        .expect("failed to create page");

    let _ = page
        .goto("https://developer.mozilla.org/en-US/docs/Web/HTML/Element/iframe")
        .await
        .expect("failed to navigate");
}
