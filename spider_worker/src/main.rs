use spider::{tokio, utils, website::Website};
use std::convert::Infallible;
use warp::{path::FullPath, Filter};

#[macro_use]
extern crate lazy_static;

lazy_static! {
    /// top level request client to re-use
    static ref CLIENT: spider::Client = {
        #[allow(unused_mut)]
        let mut proxy_website = Website::new("proxy");

        utils::connect::init_background_runtime();

        proxy_website.configure_http_client()
    };
}

use hyper_util::service::TowerToHyperService;
use std::net::SocketAddr;

/// Serve warp routes on a TCP listener using hyper-util.
async fn serve_plain(routes: warp::filters::BoxedFilter<(impl warp::Reply + 'static,)>, port: u16) {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");
    let svc = TowerToHyperService::new(warp::service(routes));

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        let svc = svc.clone();
        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(stream);
            let _ =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(io, svc)
                    .await;
        });
    }
}

/// Serve warp routes over TLS using tokio-rustls.
#[cfg(feature = "tls")]
async fn serve_tls(
    routes: warp::filters::BoxedFilter<(impl warp::Reply + 'static,)>,
    port: u16,
    cert_path: &str,
    key_path: &str,
) {
    use std::sync::Arc;
    use tokio_rustls::TlsAcceptor;

    let certs = {
        let file = std::fs::File::open(cert_path).expect("failed to open cert file");
        let mut reader = std::io::BufReader::new(file);
        rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .expect("failed to read certs")
    };

    let key = {
        let file = std::fs::File::open(key_path).expect("failed to open key file");
        let mut reader = std::io::BufReader::new(file);
        rustls_pemfile::private_key(&mut reader)
            .expect("failed to read private key")
            .expect("no private key found")
    };

    let tls_config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("invalid TLS config");

    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind");
    let svc = TowerToHyperService::new(warp::service(routes));

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        let acceptor = tls_acceptor.clone();
        let svc = svc.clone();
        tokio::spawn(async move {
            if let Ok(tls_stream) = acceptor.accept(stream).await {
                let io = hyper_util::rt::TokioIo::new(tls_stream);
                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection(io, svc)
                .await;
            }
        });
    }
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
        page::build,
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

    let (subdomains, tld) = match referer {
        Some(r) => (r == "3" || r == "1", r == "3" || r == "2"),
        _ => (false, false),
    };

    let mut page = build("", Default::default());

    let extracted = {
        let mut selectors = spider::page::get_page_selectors(&url_path, subdomains, tld);

        let mut links: spider::hashbrown::HashSet<spider::CaseInsensitiveString> =
            spider::hashbrown::HashSet::new();

        page.clone_from(
            &spider::page::Page::new_page_streaming(
                &url_path,
                &CLIENT,
                false,
                &mut selectors,
                &Default::default(),
                &Default::default(),
                &mut links,
                None,
                &None,
                &mut None,
                &mut None,
            )
            .await,
        );

        let mut s = flexbuffers::FlexbufferSerializer::new();

        let _ = links.serialize(&mut s);

        s.take_buffer()
    };

    #[cfg(feature = "headers")]
    /// Return the response with the header information.
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
                WorkerProxyHeaderBuilder::new()
            };

            builder.set_status_code(page.status_code.as_u16());
            match response.headers_mut() {
                Some(headers) => {
                    let h = builder.build();

                    headers.extend(h.into_iter().filter_map(|(key, value)| {
                        if let Some(name) = key {
                            let header_name =
                                warp::http::HeaderName::from_bytes(name.as_str().as_bytes())
                                    .ok()?;
                            let header_value =
                                warp::http::HeaderValue::from_str(value.to_str().ok()?).ok()?;
                            Some((Some(header_name), header_value))
                        } else {
                            None
                        }
                    }));
                }
                _ => (),
            }
        }
        Ok(response
            .status(StatusCode::OK)
            .body(extracted)
            .unwrap_or_else(|_| warp::http::Response::new(Vec::new())))
    }

    #[cfg(not(feature = "headers"))]
    /// Return the response.
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

    let data = utils::fetch_page_html_raw(&url_path, &CLIENT).await;

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

            if let Some(headers) = response.headers_mut() {
                let h = builder.build();

                headers.extend(h.into_iter().filter_map(|(key, value)| {
                    if let Some(name) = key {
                        let header_name =
                            warp::http::HeaderName::from_bytes(name.as_str().as_bytes()).ok()?;
                        let header_value =
                            warp::http::HeaderValue::from_str(value.to_str().ok()?).ok()?;
                        Some((Some(header_name), header_value))
                    } else {
                        None
                    }
                }));
            }
        }
        Ok(response
            .status(StatusCode::OK)
            .body(data.content.unwrap_or_default().to_vec())
            .unwrap_or_else(|_| warp::http::Response::new(Vec::new())))
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

    serve_plain(routes, port).await;
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

    serve_plain(routes, port).await;
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

        serve_plain(routes, port).await;
    });

    let port: u16 = std::env::var("SPIDER_WORKER_PORT")
        .unwrap_or_else(|_| "3030".into())
        .parse()
        .unwrap_or_else(|_| 3030);
    utils::log("Spider_Worker starting at 0.0.0.0:", &port.to_string());

    serve_plain(routes, port).await;
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

    serve_tls(routes, port, &pem_cert, &rsa_key).await;
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
        .unwrap_or(3031);

    utils::log("Spider_Worker starting at 0.0.0.0:", port.to_string());

    let pem_cert: String =
        std::env::var("SPIDER_WORKER_CERT_PATH").unwrap_or_else(|_| "/cert.pem".into());
    let rsa_key: String =
        std::env::var("SPIDER_WORKER_KEY_PATH").unwrap_or_else(|_| "/key.rsa".into());

    serve_tls(routes, port, &pem_cert, &rsa_key).await;
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

        serve_plain(routes, port).await;
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

    serve_tls(routes, port, &pem_cert, &rsa_key).await;
}
