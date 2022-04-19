pub use crate::reqwest::{Client, Error};

#[tokio::main]
pub async fn fetch_page_html(url: &str, client: &Client) -> Result<String, Error> {
    let body = client.get(url).send().await?.text().await?;

    Ok(body)
}
