/// The asynchronous Client to make requests with.
#[cfg(all(not(feature = "cache_request"), not(feature = "rquest")))]
pub type Client = reqwest::Client;
#[cfg(all(not(feature = "cache_request"), not(feature = "rquest")))]
/// The asynchronous Client Builder.
pub type ClientBuilder = reqwest::ClientBuilder;
#[cfg(all(not(feature = "cache_request"), not(feature = "rquest")))]
pub use reqwest as request_client;
#[cfg(all(not(feature = "cache_request"), not(feature = "rquest")))]
pub use reqwest::{header, redirect, Error, Proxy, Response};

/// The asynchronous Client to make requests with rquest.
#[cfg(all(not(feature = "cache_request"), feature = "rquest"))]
pub type Client = rquest::Client;
#[cfg(all(not(feature = "cache_request"), feature = "rquest"))]
/// The asynchronous Client Builder.
pub type ClientBuilder = rquest::ClientBuilder;
#[cfg(all(not(feature = "cache_request"), feature = "rquest"))]
pub use rquest as request_client;
#[cfg(all(not(feature = "cache_request"), feature = "rquest"))]
pub use rquest::{header, redirect, Error, Proxy, Response};
#[cfg(all(not(feature = "cache_request"), feature = "rquest"))]
pub use rquest_util;

/// The asynchronous Client to make requests with HTTP Cache.
#[cfg(feature = "cache_request")]
pub type Client = reqwest_middleware::ClientWithMiddleware;
#[cfg(feature = "cache_request")]
/// The asynchronous Client Builder.
pub type ClientBuilder = reqwest_middleware::ClientBuilder;
#[cfg(feature = "cache_request")]
pub use reqwest::{header, redirect, Proxy, Response};
#[cfg(feature = "cache_request")]
pub use reqwest_middleware as request_client;
#[cfg(feature = "cache_request")]
pub use reqwest_middleware::Error;
