use std::convert::Infallible;

use spider::{tokio, utils::fetch_page_html, website::Website};
use warp::{path::FullPath, Filter};

#[macro_use]
extern crate lazy_static;

lazy_static! {
    /// top level request client to re-use
    static ref CLIENT: spider::reqwest::Client = {
        let mut proxy_website = Website::new("proxy");
        let client = proxy_website.configure_http_client();

        client
    };
}

// forward request to get resources
async fn forward(path: FullPath, host: String) -> Result<impl warp::Reply, Infallible> {
    let data = fetch_page_html(
        &format!(
            "{}://{}{}",
            if host.ends_with("443") {
                "https"
            } else {
                "http"
            },
            host,
            path.as_str()
        ),
        &CLIENT,
    )
    .await;

    Ok(data.unwrap_or_default())
}

#[tokio::main]
async fn main() {
    let host = warp::header::<String>("host");
    let routes = warp::path::full().and(host).and_then(forward).boxed();

    warp::serve(routes).run(([0, 0, 0, 0], 3030)).await;
}
