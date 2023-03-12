use std::convert::Infallible;

use spider::{tokio, website::Website};
use warp::{path::FullPath, Filter};

#[macro_use]
extern crate lazy_static;

lazy_static! {
    /// top level request client to re-use
    static ref CLIENT: spider::reqwest::Client = {
        let mut proxy_website = Website::new("proxy");
        let client = proxy_website.configure_http_client(false);

        client
    };
}

/// convert the data into vec bytes
#[cfg(not(feature = "scrape"))]
fn serialize(set: spider::hashbrown::HashSet<warp::hyper::body::Bytes>) -> Vec<u8> {
    let cap = set.len();
    let cap = cap * 2;

    set.into_iter().fold(Vec::with_capacity(cap), |mut acc, v| {
        acc.extend_from_slice(&v);
        acc.extend_from_slice(" ".as_bytes());

        acc
    })
}

/// forward request to get resources
#[cfg(not(feature = "scrape"))]
async fn forward(
    path: FullPath,
    host: String,
    referer: Option<String>,
) -> Result<impl warp::Reply, Infallible> {
    use spider::string_concat::{string_concat, string_concat_impl};

    let url = &string_concat!(
        if host.ends_with("443") {
            "https"
        } else {
            "http"
        },
        "://",
        host,
        path.as_str()
    );

    let page = spider::page::Page::new(&url, &CLIENT).await;

    if !page.get_html().is_empty() {
        let (subdomains, tld) = match referer {
            Some(r) => (r == "3" || r == "1", r == "3" || r == "2"),
            _ => (false, false),
        };
        let selectors = spider::page::get_raw_selectors(url, subdomains, tld).unwrap();
        let links = page
            .links_stream::<warp::hyper::body::Bytes>(&(&selectors.0, &selectors.1))
            .await;

        Ok(serialize(links))
    } else {
        Ok(Default::default())
    }
}

/// forward request to get links resources
#[cfg(not(all(not(feature = "scrape"), not(feature = "all"))))]
async fn scrape(path: FullPath, host: String) -> Result<impl warp::Reply, Infallible> {
    let data = spider::utils::fetch_page_html(
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
#[cfg(all(not(feature = "scrape"), not(feature = "all")))]
async fn main() {
    let host = warp::header::<String>("host");
    let referer = warp::header::optional::<String>("referer");
    let routes = warp::path::full()
        .and(host)
        .and(referer)
        .and_then(forward)
        .boxed();

    warp::serve(routes).run(([0, 0, 0, 0], 3030)).await;
}

#[tokio::main]
#[cfg(feature = "scrape")]
async fn main() {
    let host = warp::header::<String>("host");
    let routes = warp::path::full().and(host).and_then(scrape).boxed();

    warp::serve(routes).run(([0, 0, 0, 0], 3031)).await;
}

#[tokio::main]
#[cfg(feature = "all")]
async fn main() {
    let host = warp::header::<String>("host");
    let referer = warp::header::optional::<String>("referer");
    let routes = warp::path::full()
        .and(host)
        .and(referer)
        .and_then(forward)
        .boxed();

    tokio::spawn(async {
        let host = warp::header::<String>("host");
        let routes = warp::path::full().and(host).and_then(scrape).boxed();

        warp::serve(routes).run(([0, 0, 0, 0], 3031)).await;
    });

    warp::serve(routes).run(([0, 0, 0, 0], 3030)).await;
}
