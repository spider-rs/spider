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
use spider::tokio::sync::Semaphore;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
#[cfg(feature = "tls")]
use std::time::Duration;

fn env_value(name: &str) -> io::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name}: {error}"),
        )),
    }
}

fn parse_port(name: &str, value: Option<&str>, default: u16) -> io::Result<u16> {
    match value {
        None => Ok(default),
        Some(value) => value.parse().map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("invalid {name}: {e}"))
        }),
    }
}

fn worker_port(name: &str, default: u16) -> io::Result<u16> {
    parse_port(name, env_value(name)?.as_deref(), default)
}

/// Per-listener cap on concurrent in-flight connections. Without a bound the
/// accept loop spawns an unbounded task (and up to ~1 GiB fetch) per connection,
/// so a burst of connections can exhaust tasks / FDs / memory. Opt-in and
/// **default `0` (unlimited)**; set
/// `SPIDER_WORKER_MAX_CONCURRENCY` to a positive value to enable the bound.
fn connection_semaphore() -> io::Result<Arc<Semaphore>> {
    semaphore_from_config(env_value("SPIDER_WORKER_MAX_CONCURRENCY")?.as_deref())
}

fn semaphore_from_config(value: Option<&str>) -> io::Result<Arc<Semaphore>> {
    let limit = match value {
        None => 0,
        Some(value) => value.parse::<usize>().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid SPIDER_WORKER_MAX_CONCURRENCY: {e}"),
            )
        })?,
    };
    // `0` = unlimited; clamp to MAX_PERMITS so an oversized env value can't make
    // `Semaphore::new` panic.
    let permits = if limit == 0 {
        Semaphore::MAX_PERMITS
    } else {
        limit.min(Semaphore::MAX_PERMITS)
    };
    Ok(Arc::new(Semaphore::new(permits)))
}

/// Shed excess connections immediately. No accepted socket can hold the only
/// accept loop waiting for capacity, and no task is spawned without a permit.
fn admit_connection(
    sem: Arc<Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::TryAcquireError> {
    sem.try_acquire_owned()
}

fn bind_result<T>(addr: SocketAddr, result: io::Result<T>) -> io::Result<T> {
    result.map_err(|e| io::Error::new(e.kind(), format!("binding worker listener {addr}: {e}")))
}

#[cfg(feature = "tls")]
const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(feature = "tls")]
async fn complete_handshake<T>(
    handshake: impl std::future::Future<Output = io::Result<T>>,
    permit: tokio::sync::OwnedSemaphorePermit,
    timeout: Duration,
) -> io::Result<(T, tokio::sync::OwnedSemaphorePermit)> {
    let stream = tokio::time::timeout(timeout, handshake)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "worker TLS handshake timed out"))??;
    // Ownership passes to the HTTP connection only after a successful handshake.
    // Failure, timeout or cancellation drops the permit and the handshake future.
    Ok((stream, permit))
}

/// Best-effort SSRF guard. When `SPIDER_WORKER_BLOCK_PRIVATE_HOSTS` is enabled,
/// reject fetch targets that resolve to loopback / private / link-local literals
/// so a crafted `Host` header can't make the worker fetch internal endpoints.
/// Default off, so trusted internal-network deployments are unchanged. Note: this
/// checks literal IPs and `localhost` only — it does not resolve hostnames (no
/// DNS-rebinding protection); pair it with network-level egress controls.
fn target_host_blocked(host: &str) -> bool {
    let enabled = std::env::var("SPIDER_WORKER_BLOCK_PRIVATE_HOSTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        return false;
    }
    let h = host.trim();
    let h = h
        .strip_prefix("http://")
        .or_else(|| h.strip_prefix("https://"))
        .unwrap_or(h);
    let h = h.split('/').next().unwrap_or(h);
    // Strip an optional :port, and IPv6 brackets.
    let hostname = if h.starts_with('[') {
        h.trim_start_matches('[').split(']').next().unwrap_or(h)
    } else {
        h.rsplit_once(':').map(|(a, _)| a).unwrap_or(h)
    };
    if hostname.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match hostname.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
        }
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback() || v6.is_unspecified(),
        Err(_) => false,
    }
}

/// Serve warp routes on a TCP listener using hyper-util.
async fn serve_plain(
    routes: warp::filters::BoxedFilter<(impl warp::Reply + 'static,)>,
    port: u16,
) -> io::Result<()> {
    let sem = connection_semaphore()?;
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = bind_result(addr, tokio::net::TcpListener::bind(addr).await)?;
    let svc = TowerToHyperService::new(warp::service(routes));

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        let permit = match admit_connection(sem.clone()) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let svc = svc.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let io = hyper_util::rt::TokioIo::new(stream);
            let _ =
                hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                    .serve_connection(io, svc)
                    .await;
        });
    }
}

/// Parse TLS material without global crypto-provider selection (which can panic
/// when feature unification enables multiple providers).
#[cfg(feature = "tls")]
fn tls_config(
    certs: &mut dyn io::BufRead,
    key: &mut dyn io::BufRead,
) -> io::Result<tokio_rustls::rustls::ServerConfig> {
    use tokio_rustls::rustls;
    let invalid = |context: &str, error: String| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("worker TLS {context}: {error}"),
        )
    };
    let certs = rustls_pemfile::certs(certs)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid("certificate PEM", e.to_string()))?;
    if certs.is_empty() {
        return Err(invalid("certificate PEM", "no certificates found".into()));
    }
    let key = rustls_pemfile::private_key(key)
        .map_err(|e| invalid("private key PEM", e.to_string()))?
        .ok_or_else(|| invalid("private key PEM", "no private key found".into()))?;
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
    rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| invalid("protocol configuration", e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| invalid("certificate/key configuration", e.to_string()))
}

#[cfg(feature = "tls")]
fn load_tls_config(
    cert_path: &str,
    key_path: &str,
) -> io::Result<tokio_rustls::rustls::ServerConfig> {
    let open = |path: &str, kind: &str| {
        std::fs::File::open(path)
            .map(io::BufReader::new)
            .map_err(|e| io::Error::new(e.kind(), format!("opening worker TLS {kind} {path}: {e}")))
    };
    tls_config(
        &mut open(cert_path, "certificate")?,
        &mut open(key_path, "private key")?,
    )
}

/// Serve warp routes over TLS using tokio-rustls.
#[cfg(feature = "tls")]
async fn serve_tls(
    routes: warp::filters::BoxedFilter<(impl warp::Reply + 'static,)>,
    port: u16,
    cert_path: &str,
    key_path: &str,
) -> io::Result<()> {
    let sem = connection_semaphore()?;
    let tls_acceptor =
        tokio_rustls::TlsAcceptor::from(Arc::new(load_tls_config(cert_path, key_path)?));
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = bind_result(addr, tokio::net::TcpListener::bind(addr).await)?;
    let svc = TowerToHyperService::new(warp::service(routes));

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        let permit = match admit_connection(sem.clone()) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let acceptor = tls_acceptor.clone();
        let svc = svc.clone();
        tokio::spawn(async move {
            if let Ok((tls_stream, _permit)) =
                complete_handshake(acceptor.accept(stream), permit, TLS_HANDSHAKE_TIMEOUT).await
            {
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

    // Opt-in SSRF guard: skip the outbound fetch for blocked (internal) targets.
    let extracted = if target_host_blocked(&host) {
        Vec::new()
    } else {
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
                (None, None),
            )
            .await,
        );

        let mut s = flexbuffers::FlexbufferSerializer::new();

        let _ = links.serialize(&mut s);

        s.take_buffer()
    };

    #[cfg(feature = "headers")]
    /// Return the response with the header information.
    fn pack(
        mut page: spider::page::Page,
        extracted: Vec<u8>,
    ) -> Result<impl warp::Reply, Infallible> {
        use spider::features::decentralized_headers::WorkerProxyHeaderBuilder;
        use warp::http::{Response, StatusCode};

        let mut response = Response::builder();
        {
            let mut builder = if let Some(headers) = page.headers.take() {
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

                    append_response_headers(headers, &h);
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

    // Opt-in SSRF guard: skip the outbound fetch for blocked (internal) targets.
    let data = if target_host_blocked(&host) {
        Default::default()
    } else {
        utils::fetch_page_html_raw(&url_path, &CLIENT).await
    };

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

                append_response_headers(headers, &h);
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

/// Copy named values so repeated and opaque upstream values survive packing.
#[cfg(feature = "headers")]
fn append_response_headers(
    target: &mut warp::http::HeaderMap,
    source: &spider::reqwest::header::HeaderMap,
) {
    for (name, value) in source {
        if let (Ok(name), Ok(value)) = (
            warp::http::HeaderName::from_bytes(name.as_str().as_bytes()),
            warp::http::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            target.append(name, value);
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::try_init()
        .map_err(|e| io::Error::other(format!("initializing worker logging: {e}")))?;

    #[cfg(feature = "scrape")]
    let (routes, port) = (
        warp::path::full()
            .and(warp::header::<String>("host"))
            .and_then(scrape)
            .boxed(),
        worker_port("SPIDER_WORKER_SCRAPER_PORT", 3031)?,
    );
    #[cfg(not(feature = "scrape"))]
    let (routes, port) = (
        warp::path::full()
            .and(warp::header::<String>("host"))
            .and(warp::header::optional::<String>("referer"))
            .and_then(forward)
            .boxed(),
        worker_port("SPIDER_WORKER_PORT", 3030)?,
    );

    utils::log("Spider_Worker starting at 0.0.0.0:", port.to_string());
    #[cfg(feature = "tls")]
    let pem_cert = env_value("SPIDER_WORKER_CERT_PATH")?.unwrap_or_else(|| "/cert.pem".into());
    #[cfg(feature = "tls")]
    let rsa_key = env_value("SPIDER_WORKER_KEY_PATH")?.unwrap_or_else(|| "/key.rsa".into());
    #[cfg(feature = "tls")]
    let server = serve_tls(routes, port, &pem_cert, &rsa_key);
    #[cfg(not(feature = "tls"))]
    let server = serve_plain(routes, port);

    #[cfg(all(feature = "full_resources", not(feature = "scrape")))]
    {
        let scrape_routes = warp::path::full()
            .and(warp::header::<String>("host"))
            .and_then(scrape)
            .boxed();
        let scrape_port = worker_port("SPIDER_WORKER_SCRAPER_PORT", 3031)?;
        utils::log(
            "Spider_Worker scraper starting at 0.0.0.0:",
            scrape_port.to_string(),
        );
        // Propagate startup failures from either listener; do not leave a detached
        // scraper running after the primary listener fails (or vice versa).
        tokio::try_join!(server, serve_plain(scrape_routes, scrape_port))?;
        Ok(())
    }
    #[cfg(not(all(feature = "full_resources", not(feature = "scrape"))))]
    server.await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_safety_connection_capacity_bounds_and_defaults() {
        for value in [None, Some("0")] {
            let sem = semaphore_from_config(value).unwrap();
            assert_eq!(sem.available_permits(), Semaphore::MAX_PERMITS);
            let permit = admit_connection(sem.clone()).unwrap();
            assert_eq!(sem.available_permits(), Semaphore::MAX_PERMITS - 1);
            drop(permit);
            assert_eq!(sem.available_permits(), Semaphore::MAX_PERMITS);
        }
        for value in [
            Semaphore::MAX_PERMITS,
            Semaphore::MAX_PERMITS + 1,
            usize::MAX,
        ] {
            let sem = semaphore_from_config(Some(&value.to_string())).unwrap();
            assert_eq!(sem.available_permits(), Semaphore::MAX_PERMITS);
        }
        for value in [
            "",
            "-1",
            "invalid",
            "999999999999999999999999999999999999999",
        ] {
            assert_eq!(
                semaphore_from_config(Some(value)).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn runtime_safety_connection_admission_sheds_full_and_closed_capacity() {
        let sem = semaphore_from_config(Some("2")).unwrap();
        let first = admit_connection(sem.clone()).unwrap();
        let second = admit_connection(sem.clone()).unwrap();
        assert!(matches!(
            admit_connection(sem.clone()),
            Err(tokio::sync::TryAcquireError::NoPermits)
        ));
        drop(first);
        let replacement = admit_connection(sem.clone()).unwrap();
        assert_eq!(sem.available_permits(), 0);
        drop((second, replacement));
        assert_eq!(sem.available_permits(), 2);
        sem.close();
        assert!(matches!(
            admit_connection(sem),
            Err(tokio::sync::TryAcquireError::Closed)
        ));
    }

    #[test]
    fn runtime_safety_invalid_port_and_bind_failure_are_errors() {
        assert_eq!(parse_port("test port", None, 3030).unwrap(), 3030);
        for (input, expected) in [("0", 0), ("3031", 3031), ("65535", 65535)] {
            assert_eq!(
                parse_port("test port", Some(input), 3030).unwrap(),
                expected
            );
        }
        for input in ["", "-1", "65536", "not-a-port"] {
            assert_eq!(
                parse_port("test port", Some(input), 3030)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        // Inject the bind failure; no socket is opened by this test.
        let error = bind_result::<()>(addr, Err(io::ErrorKind::AddrInUse.into())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert!(error
            .to_string()
            .contains("binding worker listener 127.0.0.1:0"));
    }

    #[cfg(feature = "tls")]
    struct PendingHandshake(Arc<std::sync::atomic::AtomicBool>);

    #[cfg(feature = "tls")]
    impl std::future::Future for PendingHandshake {
        type Output = io::Result<()>;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            std::task::Poll::Pending
        }
    }

    #[cfg(feature = "tls")]
    impl Drop for PendingHandshake {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[cfg(feature = "tls")]
    #[tokio::test(start_paused = true)]
    async fn runtime_safety_pending_tls_handshake_times_out_and_releases_capacity() {
        let sem = semaphore_from_config(Some("1")).unwrap();
        let permit = admit_connection(sem.clone()).unwrap();
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let start = tokio::time::Instant::now();
        let error = complete_handshake(
            PendingHandshake(dropped.clone()),
            permit,
            TLS_HANDSHAKE_TIMEOUT,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(start.elapsed(), TLS_HANDSHAKE_TIMEOUT);
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(sem.available_permits(), 1);
        assert!(admit_connection(sem).is_ok());
    }

    #[cfg(feature = "tls")]
    #[tokio::test(start_paused = true)]
    async fn runtime_safety_cancelled_tls_handshake_releases_capacity() {
        let sem = semaphore_from_config(Some("1")).unwrap();
        let permit = admit_connection(sem.clone()).unwrap();
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task = tokio::spawn(complete_handshake(
            PendingHandshake(dropped.clone()),
            permit,
            TLS_HANDSHAKE_TIMEOUT,
        ));
        tokio::task::yield_now().await;
        assert_eq!(sem.available_permits(), 0);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(sem.available_permits(), 1);
    }

    #[cfg(feature = "tls")]
    #[tokio::test(start_paused = true)]
    async fn runtime_safety_tls_success_retains_permit_and_failure_releases_it() {
        let sem = semaphore_from_config(Some("1")).unwrap();
        let permit = admit_connection(sem.clone()).unwrap();
        let (value, permit) = complete_handshake(async { Ok(42) }, permit, TLS_HANDSHAKE_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(value, 42);
        assert_eq!(sem.available_permits(), 0);
        drop(permit);
        let permit = admit_connection(sem.clone()).unwrap();
        let error = complete_handshake(
            async { Err::<(), _>(io::Error::from(io::ErrorKind::InvalidData)) },
            permit,
            TLS_HANDSHAKE_TIMEOUT,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(sem.available_permits(), 1);
    }

    #[cfg(feature = "tls")]
    #[test]
    fn runtime_safety_tls_malformed_boot_inputs() {
        // Dummy bytes only: these contain no certificate or private key material.
        const CERT: &[u8] = b"-----BEGIN CERTIFICATE-----\nAA==\n-----END CERTIFICATE-----\n";
        const KEY: &[u8] = b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----\n";
        for (cert, key, context) in [
            (&b""[..], &b""[..], "certificate PEM"),
            (
                &b"-----BEGIN CERTIFICATE-----\n%%%\n-----END CERTIFICATE-----\n"[..],
                KEY,
                "certificate PEM",
            ),
            (CERT, &b""[..], "no private key found"),
            (
                CERT,
                &b"-----BEGIN PRIVATE KEY-----\n%%%\n-----END PRIVATE KEY-----\n"[..],
                "private key PEM",
            ),
            (CERT, KEY, "certificate/key configuration"),
        ] {
            let error =
                tls_config(&mut io::Cursor::new(cert), &mut io::Cursor::new(key)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains(context), "{error}");
        }
    }

    #[cfg(feature = "tls")]
    #[test]
    fn runtime_safety_tls_missing_boot_files() {
        let root = tempfile::tempdir().unwrap();
        let cert = root.path().join("missing.pem");
        let key = root.path().join("missing.key");
        let error = load_tls_config(cert.to_str().unwrap(), key.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("opening worker TLS certificate"));
        std::fs::write(&cert, b"not a certificate").unwrap();
        let error = load_tls_config(cert.to_str().unwrap(), key.to_str().unwrap()).unwrap_err();
        assert!(error.to_string().contains("opening worker TLS private key"));
    }

    #[cfg(feature = "headers")]
    #[test]
    fn runtime_safety_worker_response_preserves_duplicate_and_opaque_headers() {
        use spider::features::decentralized_headers::WorkerProxyHeaderBuilder;
        use spider::reqwest::header::{HeaderMap, HeaderValue};
        let mut upstream = HeaderMap::new();
        upstream.append("set-cookie", HeaderValue::from_static("a=1"));
        upstream.append("set-cookie", HeaderValue::from_static("b=2"));
        upstream.append("opaque", HeaderValue::from_bytes(&[0x80]).unwrap());
        let mut builder = WorkerProxyHeaderBuilder::new();
        builder.extend(upstream);
        builder.set_status_code(201);
        let mut response = warp::http::HeaderMap::new();
        append_response_headers(&mut response, &builder.build());
        assert_eq!(
            response
                .get_all("zz-spider-r--set-cookie")
                .iter()
                .map(|v| v.as_bytes())
                .collect::<Vec<_>>(),
            vec![&b"a=1"[..], &b"b=2"[..]]
        );
        assert_eq!(response["zz-spider-r--opaque"].as_bytes(), &[0x80]);
        assert_eq!(response["zz-spider-r--status-code"], "201");
    }
}
