use futures::StreamExt;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide_cdp::cdp::browser_protocol::page::PrintToPdfParams;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let (browser, mut handler) = Browser::launch(BrowserConfig::builder().build()?).await?;

    let handle = tokio::task::spawn(async move {
        loop {
            let _ = handler.next().await.unwrap();
        }
    });

    let page = browser.new_page("https://news.ycombinator.com/").await?;

    // save the page as pdf
    page.save_pdf(PrintToPdfParams::default(), "hn.pdf").await?;

    handle.await?;
    Ok(())
}
