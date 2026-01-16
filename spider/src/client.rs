/// The asynchronous Client to make requests with.
#[cfg(all(not(feature = "cache_request"), not(feature = "wreq")))]
pub type Client = reqwest::Client;
#[cfg(all(not(feature = "cache_request"), not(feature = "wreq")))]
/// The asynchronous Client Builder.
pub type ClientBuilder = reqwest::ClientBuilder;
#[cfg(all(not(feature = "cache_request"), not(feature = "wreq")))]
pub use reqwest as request_client;
#[cfg(all(
    feature = "cookies",
    not(feature = "cache_request"),
    not(feature = "wreq")
))]
pub use reqwest::cookie;
#[cfg(all(not(feature = "cache_request"), not(feature = "wreq")))]
pub use reqwest::{header, redirect, Error, Proxy, Response, StatusCode};

/// The asynchronous Client to make requests with wreq.
#[cfg(all(not(feature = "cache_request"), feature = "wreq"))]
pub type Client = wreq::Client;
#[cfg(all(not(feature = "cache_request"), feature = "wreq"))]
/// The asynchronous Client Builder.
pub type ClientBuilder = wreq::ClientBuilder;
#[cfg(all(not(feature = "cache_request"), feature = "wreq"))]
pub use wreq as request_client;
#[cfg(all(feature = "cookies", not(feature = "cache_request"), feature = "wreq"))]
pub use wreq::cookie;
#[cfg(all(not(feature = "cache_request"), feature = "wreq"))]
pub use wreq::{header, redirect, Error, Proxy, Response, StatusCode};
#[cfg(all(not(feature = "cache_request"), feature = "wreq"))]
pub use wreq_util;

/// The asynchronous Client to make requests with HTTP Cache.
#[cfg(feature = "cache_request")]
pub type Client = reqwest_middleware::ClientWithMiddleware;

#[cfg(feature = "cache_request")]
/// The asynchronous Client Builder.
pub type ClientBuilder = reqwest::ClientBuilder;

#[cfg(all(feature = "cookies", feature = "cache_request"))]
pub use reqwest::cookie;
#[cfg(feature = "cache_request")]
pub use reqwest::{header, redirect, Proxy, Response, StatusCode};
#[cfg(feature = "cache_request")]
pub use reqwest_middleware as request_client;
#[cfg(feature = "cache_request")]
pub use reqwest_middleware::Error;
