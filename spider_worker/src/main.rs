use std::convert::Infallible;

use spider::{tokio, utils::log, website::Website};
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

/// forward request to get resources
#[cfg(not(feature = "scrape"))]
async fn forward(
    path: FullPath,
    host: String,
    referer: Option<String>,
) -> Result<impl warp::Reply, Infallible> {
    use spider::{
        flexbuffers,
        serde::Serialize,
        string_concat::{string_concat, string_concat_impl},
    };

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

    Ok(if !page.get_html().is_empty() {
        let (subdomains, tld) = match referer {
            Some(r) => (r == "3" || r == "1", r == "3" || r == "2"),
            _ => (false, false),
        };

        match spider::page::get_page_selectors(url, subdomains, tld) {
            Some(selectors) => {
                let links = page
                    .links_stream::<spider::bytes::Bytes>(&(&selectors.0, &selectors.1))
                    .await;

                let mut s = flexbuffers::FlexbufferSerializer::new();

                match links.serialize(&mut s) {
                    _ => (),
                };

                s.take_buffer()
            }
            _ => Default::default(),
        }
    } else {
        Default::default()
    })
}

/// forward request to get links resources
#[cfg(not(all(not(feature = "scrape"), not(feature = "all"))))]
async fn scrape(path: FullPath, host: String) -> Result<impl warp::Reply, Infallible> {
    let data = utils::fetch_page_html(
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
    env_logger::init();
    let host = warp::header::<String>("host");
    let referer = warp::header::optional::<String>("referer");
    let routes = warp::path::full()
        .and(host)
        .and(referer)
        .and_then(forward)
        .boxed();
    let port: u16 = std::env::var("SPIDER_WORKER_PORT")
        .unwrap_or_else(|_| "3030".into())
        .parse()
        .unwrap_or_else(|_| 3030);
    log("Spider_Worker starting at 0.0.0.0:", &port.to_string());

    warp::serve(routes).run(([0, 0, 0, 0], port)).await;
}

#[tokio::main]
#[cfg(feature = "scrape")]
async fn main() {
    env_logger::init();
    let host = warp::header::<String>("host");
    let routes = warp::path::full().and(host).and_then(scrape).boxed();
    let port: u16 = std::env::var("SPIDER_WORKER_SCRAPER_PORT")
        .unwrap_or_else(|_| "3031".into())
        .parse()
        .unwrap_or_else(|_| 3031);
    log(
        "Spider_Worker scraper starting at 0.0.0.0:",
        &port.to_string(),
    );

    warp::serve(routes).run(([0, 0, 0, 0], port)).await;
}

#[tokio::main]
#[cfg(feature = "all")]
async fn main() {
    env_logger::init();
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
        let port: u16 = std::env::var("SPIDER_WORKER_SCRAPER_PORT")
            .unwrap_or_else(|_| "3031".into())
            .parse()
            .unwrap_or_else(|_| 3031);

        log("Spider_Worker scraper starting at 0.0.0.0:", &port);

        warp::serve(routes).run(([0, 0, 0, 0], port)).await;
    });

    let port: u16 = std::env::var("SPIDER_WORKER_PORT")
        .unwrap_or_else(|_| "3030".into())
        .parse()
        .unwrap_or_else(|_| 3030);
    log("Spider_Worker starting at 0.0.0.0:", &port);

    warp::serve(routes).run(([0, 0, 0, 0], port)).await;
}
