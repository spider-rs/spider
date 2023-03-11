use std::convert::Infallible;

use spider::{
    hashbrown::HashSet,
    page::{get_raw_selectors, Page},
    tokio,
    website::Website,
};
use warp::{hyper::body::Bytes, path::FullPath, Filter};

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

/// convert the data into vec bytes
fn serialize(set: HashSet<Bytes>) -> Vec<u8> {
    let cap = set.len();
    let cap = cap * 2;

    set.into_iter().fold(Vec::with_capacity(cap), |mut acc, v| {
        acc.extend_from_slice(&v);
        acc.extend_from_slice(" ".as_bytes());

        acc
    })
}

// forward request to get resources
async fn forward(path: FullPath, host: String) -> Result<impl warp::Reply, Infallible> {
    let url = &format!(
        "{}://{}{}",
        if host.ends_with("443") {
            "https"
        } else {
            "http"
        },
        host,
        path.as_str()
    );

    let page = Page::new(&url, &CLIENT).await;

    if !page.get_html().is_empty() {
        let selectors = get_raw_selectors(url, false, false).unwrap();
        let links = page
            .links_stream::<Bytes>(&(&selectors.0, &selectors.1))
            .await;

        Ok(serialize(links))
    } else {
        Ok(Default::default())
    }
}

#[tokio::main]
async fn main() {
    let host = warp::header::<String>("host");
    let routes = warp::path::full().and(host).and_then(forward).boxed();

    warp::serve(routes).run(([0, 0, 0, 0], 3030)).await;
}
