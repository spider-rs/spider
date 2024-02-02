use spider::{tokio, utils, website::Website};
use std::convert::Infallible;
use warp::{path::FullPath, Filter};

#[macro_use]
extern crate lazy_static;

lazy_static! {
    /// top level request client to re-use
    static ref CLIENT: spider::Client = {
        let mut proxy_website = Website::new("proxy");

        proxy_website.configure_http_client()
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

    let url_path = if host.starts_with("http") {
        string_concat!(host, path.as_str())
    } else {
        string_concat!(
            if host.ends_with("443") {
                "https"
            } else {
                "http"
            },
            "://",
            host,
            path.as_str()
        )
    };

    let page = spider::page::Page::new_page(&url_path, &CLIENT).await;

    let extracted = if !page.get_html().is_empty() {
        let (subdomains, tld) = match referer {
            Some(r) => (r == "3" || r == "1", r == "3" || r == "2"),
            _ => (false, false),
        };

        match spider::page::get_page_selectors(&url_path, subdomains, tld) {
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
    };

    #[cfg(feature = "headers")]
    fn pack(page: spider::page::Page, extracted: Vec<u8>) -> Result<impl warp::Reply, Infallible> {
        use spider::features::decentralized_headers::WorkerProxyHeaderBuilder;
        use warp::http::{Response, StatusCode};

        let mut response = Response::builder();
        {
            let mut builder = if let Some(headers) = page.headers {
                let mut builder = WorkerProxyHeaderBuilder::with_capacity(headers.len() + 1);
                builder.extend(headers);
                builder
            } else {
                WorkerProxyHeaderBuilder::new();
            };
            builder.set_status_code(page.status_code.as_u16());
            response.headers_mut().unwrap().extend(builder.build());
        }
        Ok(response.status(StatusCode::OK).body(extracted).unwrap())
    }

    #[cfg(not(feature = "headers"))]
    fn pack(_page: spider::page::Page, extracted: Vec<u8>) -> Result<impl warp::Reply, Infallible> {
        Ok(extracted)
    }

    pack(page, extracted)
}

/// forward request to get links resources
#[cfg(not(all(not(feature = "scrape"), not(feature = "full_resources"))))]
async fn scrape(path: FullPath, host: String) -> Result<impl warp::Reply, Infallible> {
    use spider::string_concat::{string_concat, string_concat_impl};

    let url_path = if host.starts_with("http") {
        string_concat!(host, path.as_str())
    } else {
        string_concat!(
            if host.ends_with("443") {
                "https"
            } else {
                "http"
            },
            "://",
            host,
            path.as_str()
        )
    };
    let data = utils::fetch_page_html(&url_path, &CLIENT).await;

    #[cfg(feature = "headers")]
    fn pack(data: spider::utils::PageResponse) -> Result<impl warp::Reply, Infallible> {
        use spider::features::decentralized_headers::WorkerProxyHeaderBuilder;
        use warp::http::{Response, StatusCode};

        let mut response = Response::builder();
        {
            let mut builder = if let Some(headers) = data.headers {
                let mut builder = WorkerProxyHeaderBuilder::with_capacity(headers.len() + 1);
                builder.extend(headers);
                builder
            } else {
                WorkerProxyHeaderBuilder::new()
            };
            builder.set_status_code(data.status_code.as_u16());
            response.headers_mut().unwrap().extend(builder.build());
        }
        Ok(response
            .status(StatusCode::OK)
            .body(data.content.unwrap_or_default().to_vec())
            .unwrap())
    }

    #[cfg(not(feature = "headers"))]
    fn pack(data: spider::utils::PageResponse) -> Result<impl warp::Reply, Infallible> {
        Ok(data.content.unwrap_or_default().to_vec())
    }

    pack(data)
}

#[tokio::main]
#[cfg(all(
    not(feature = "scrape"),
    not(feature = "full_resources"),
    not(feature = "tls")
))]
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
        .unwrap_or(3030);

    utils::log("Spider_Worker starting at 0.0.0.0:", port.to_string());

    warp::serve(routes).run(([0, 0, 0, 0], port)).await;
}

#[tokio::main]
#[cfg(all(feature = "scrape", not(feature = "tls"),))]
async fn main() {
    env_logger::init();
    let host = warp::header::<String>("host");
    let routes = warp::path::full().and(host).and_then(scrape).boxed();
    let port: u16 = std::env::var("SPIDER_WORKER_SCRAPER_PORT")
        .unwrap_or_else(|_| "3031".into())
        .parse()
        .unwrap_or_else(|_| 3031);

    utils::log("Spider_Worker starting at 0.0.0.0:", &port.to_string());

    warp::serve(routes).run(([0, 0, 0, 0], port)).await;
}

#[tokio::main]
#[cfg(all(
    feature = "full_resources",
    not(feature = "tls"),
    not(feature = "scrape"),
))]
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

        utils::log(
            "Spider_Worker scraper starting at 0.0.0.0:",
            &port.to_string(),
        );

        warp::serve(routes).run(([0, 0, 0, 0], port)).await;
    });

    let port: u16 = std::env::var("SPIDER_WORKER_PORT")
        .unwrap_or_else(|_| "3030".into())
        .parse()
        .unwrap_or_else(|_| 3030);
    utils::log("Spider_Worker starting at 0.0.0.0:", &port.to_string());

    warp::serve(routes).run(([0, 0, 0, 0], port)).await;
}

// tls handling

#[tokio::main]
#[cfg(all(
    not(feature = "scrape"),
    not(feature = "full_resources"),
    feature = "tls"
))]
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

    utils::log("Spider_Worker starting at 0.0.0.0:", &port.to_string());

    let pem_cert: String =
        std::env::var("SPIDER_WORKER_CERT_PATH").unwrap_or_else(|_| "/cert.pem".into());
    let rsa_key: String =
        std::env::var("SPIDER_WORKER_KEY_PATH").unwrap_or_else(|_| "/key.rsa".into());

    warp::serve(routes)
        .tls()
        .cert_path(pem_cert)
        .key_path(rsa_key)
        .run(([0, 0, 0, 0], port))
        .await;
}

#[tokio::main]
#[cfg(all(feature = "scrape", feature = "tls"))]
async fn main() {
    env_logger::init();
    let host = warp::header::<String>("host");
    let routes = warp::path::full().and(host).and_then(scrape).boxed();
    let port: u16 = std::env::var("SPIDER_WORKER_SCRAPER_PORT")
        .unwrap_or_else(|_| "3031".into())
        .parse()
        .unwrap_or_else(|_| 3031);

    utils::log("Spider_Worker starting at 0.0.0.0:", &port.to_string());

    let pem_cert: String =
        std::env::var("SPIDER_WORKER_CERT_PATH").unwrap_or_else(|_| "/cert.pem".into());
    let rsa_key: String =
        std::env::var("SPIDER_WORKER_KEY_PATH").unwrap_or_else(|_| "/key.rsa".into());

    warp::serve(routes)
        .tls()
        .cert_path(pem_cert)
        .key_path(rsa_key)
        .run(([0, 0, 0, 0], port))
        .await;
}

#[tokio::main]
#[cfg(all(not(feature = "scrape"), feature = "full_resources", feature = "tls"))]
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

        utils::log(
            "Spider_Worker scraper starting at 0.0.0.0:",
            &port.to_string(),
        );

        warp::serve(routes).run(([0, 0, 0, 0], port)).await;
    });

    let port: u16 = std::env::var("SPIDER_WORKER_PORT")
        .unwrap_or_else(|_| "3030".into())
        .parse()
        .unwrap_or_else(|_| 3030);

    utils::log("Spider_Worker starting at 0.0.0.0:", &port.to_string());

    let pem_cert: String =
        std::env::var("SPIDER_WORKER_CERT_PATH").unwrap_or_else(|_| "/cert.pem".into());
    let rsa_key: String =
        std::env::var("SPIDER_WORKER_KEY_PATH").unwrap_or_else(|_| "/key.rsa".into());

    warp::serve(routes)
        .tls()
        .cert_path(pem_cert)
        .key_path(rsa_key)
        .run(([0, 0, 0, 0], port))
        .await;
}
