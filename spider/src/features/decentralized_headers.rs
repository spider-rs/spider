use itertools::{Either, Itertools};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::str::FromStr;

/// The prefix of all header values retrieved & proxied by a worker
pub const WORKER_PROXY_HEADER_PREFIX: &str = "zz-spider-r--";

/// The header name for a retrieved status code
pub const STATUS_CODE_HEADER_FIELD: HeaderName = HeaderName::from_static("status-code");

/// Shortcut for a proxied status-code.
const PROXIED_ORIGINAL_STATUS: HeaderName = HeaderName::from_static(const_format::concatcp!(
    WORKER_PROXY_HEADER_PREFIX,
    "status-code"
));

/// A helper function for adding the [WORKER_PROXY_HEADER_PREFIX] prefix to [name].
fn set_prefix(name: impl AsRef<str>) -> Option<HeaderName> {
    let key_value = name.as_ref();
    let mut new_value = String::with_capacity(WORKER_PROXY_HEADER_PREFIX.len() + key_value.len());
    new_value.push_str(WORKER_PROXY_HEADER_PREFIX);
    new_value.push_str(key_value);

    HeaderName::try_from(new_value).ok()
}

/// A helper function to strip [WORKER_PROXY_HEADER_PREFIX] of the [name].
/// Returns None if there is no prefix or the suffix is not a valid header name.
pub fn strip_prefix(name: impl AsRef<str>) -> Option<HeaderName> {
    name.as_ref()
        .strip_prefix(WORKER_PROXY_HEADER_PREFIX)
        .and_then(|stripped| HeaderName::from_str(stripped).ok())
}

/// A helper function to check if [name] starts with a [WORKER_PROXY_HEADER_PREFIX].
pub fn has_prefix(name: impl AsRef<str>) -> bool {
    name.as_ref().starts_with(WORKER_PROXY_HEADER_PREFIX)
}

/// A primitive builder for a proxied header.
/// All registered entries are prefixed by
/// [WORKER_PROXY_HEADER_PREFIX]
pub struct WorkerProxyHeaderBuilder<T = HeaderValue> {
    headers: Vec<(HeaderName, T)>,
    status_code: Option<T>,
}

impl Default for WorkerProxyHeaderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerProxyHeaderBuilder {
    /// Creates a new builder for a proxied header of a worker.
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            status_code: None,
        }
    }
}

impl<T> WorkerProxyHeaderBuilder<T> {
    /// Initializes the builder with a specific size.
    pub fn with_capacity(capacity: usize) -> Self {
        WorkerProxyHeaderBuilder {
            headers: Vec::with_capacity(capacity),
            status_code: None,
        }
    }

    /// Adds an entry with ([key], [value]) to the builder.
    pub fn insert<V: Into<T>>(&mut self, key: HeaderName, value: V) {
        self.headers.push((key, value.into()))
    }

    /// Adds an [entry] to the builder.
    pub fn push(&mut self, entry: (HeaderName, T)) {
        self.headers.push(entry)
    }

    /// Sets the [status_code] to be stored in the proxied header.
    /// Returns the old value if one exists.
    pub fn set_status_code<S: Into<T>>(&mut self, status_code: S) -> Option<T> {
        self.status_code.replace(status_code.into())
    }

    /// Writes the content of self into [target]. The
    pub fn write_to(self, target: &mut HeaderMap<T>) {
        for (k, v) in self.headers {
            if let Some(name) = set_prefix(&k) {
                target.append(name, v);
            }
        }
        if let Some(status_code) = self.status_code {
            target.insert(PROXIED_ORIGINAL_STATUS, status_code);
        }
    }

    /// Builds a [HeaderMap] from the registered entries.
    pub fn build(self) -> HeaderMap<T> {
        let mut new_map = HeaderMap::with_capacity(self.headers.len());
        self.write_to(&mut new_map);
        new_map
    }
}

impl<T> Extend<(Option<HeaderName>, T)> for WorkerProxyHeaderBuilder<T> {
    fn extend<I: IntoIterator<Item = (Option<HeaderName>, T)>>(&mut self, iter: I) {
        self.headers.extend(named_headers(iter));
    }
}

/// Owned HeaderMap iteration uses None for subsequent values of the prior name.
/// Ignore malformed leading continuations, including at the start of a new call.
fn named_headers<T>(
    headers: impl IntoIterator<Item = (Option<HeaderName>, T)>,
) -> impl Iterator<Item = (HeaderName, T)> {
    headers
        .into_iter()
        .scan(None, |previous, (name, value)| {
            if name.is_some() {
                *previous = name;
            }
            Some(previous.clone().map(|name| (name, value)))
        })
        .flatten()
}

/// Prefixes all entries in [headers] with [WORKER_PROXY_HEADER_PREFIX].
/// Returns a new [HeaderMap]
pub fn as_proxy_headers<T>(headers: HeaderMap<T>) -> HeaderMap<T> {
    let mut new_headers = HeaderMap::with_capacity(0);
    extend_with_proxy_headers(&mut new_headers, headers);
    new_headers
}

/// Extends the [target] with all values from [original_header_source], where all inserted
/// entries are prefixed with [WORKER_PROXY_HEADER_PREFIX].
pub fn extend_with_proxy_headers<T, I: IntoIterator<Item = (Option<HeaderName>, T)>>(
    target: &mut HeaderMap<T>,
    proxied_headers: I,
) {
    for (key, value) in named_headers(proxied_headers) {
        if let Some(name) = set_prefix(&key) {
            target.append(name, value);
        }
    }
}

/// A splitted [HeaderMap], containing the entries for the original request and
pub struct HeaderSplit<T> {
    ///
    pub original: HashMap<HeaderName, T>,
    /// Is none if there are no
    pub proxy: HashMap<HeaderName, T>,
}

/// Splits the [header] in original and proxy. The proxy element keys are stripped from [WORKER_PROXY_HEADER_PREFIX].
pub fn split_proxy_headers<T, I: IntoIterator<Item = (HeaderName, T)>>(
    header: I,
) -> HeaderSplit<T> {
    let (a, b) = header.into_iter().partition_map(|(k, v)| {
        if let Some(stripped) = strip_prefix(&k) {
            Either::Right((stripped, v))
        } else {
            Either::Left((k, v))
        }
    });
    HeaderSplit {
        original: a,
        proxy: b,
    }
}

/// Returns true if the [headers] at least contain one element with a key that
/// has [WORKER_PROXY_HEADER_PREFIX] as prefix.
pub fn has_proxy_entries<T>(headers: &HeaderMap<T>) -> bool {
    headers.iter().any(|(k, _)| has_prefix(k))
}

/// Returns a new [HeaderMap], containing only the entries from [src] that have the [WORKER_PROXY_HEADER_PREFIX].
/// The keys in the returned map are striped from [WORKER_PROXY_HEADER_PREFIX].
pub fn extract_proxy_headers<T: Clone>(src: &HeaderMap<T>) -> HeaderMap<T> {
    let mut headers = HeaderMap::with_capacity(0);
    for (key, value) in src {
        if let Some(name) = strip_prefix(key) {
            headers.append(name, value.clone());
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_safety_owned_duplicate_headers_roundtrip() {
        let mut source = HeaderMap::new();
        source.append("set-cookie", HeaderValue::from_static("a=1"));
        source.append("set-cookie", HeaderValue::from_static("b=2"));
        source.append("vary", HeaderValue::from_static("accept"));
        source.append("vary", HeaderValue::from_static("origin"));
        let mut builder = WorkerProxyHeaderBuilder::new();
        builder.extend(source.clone());
        builder.set_status_code(201);
        let built = builder.build();
        let cleaned = extract_proxy_headers(&built);
        for name in ["set-cookie", "vary"] {
            assert_eq!(
                cleaned.get_all(name).iter().collect::<Vec<_>>(),
                source.get_all(name).iter().collect::<Vec<_>>()
            );
        }
        assert_eq!(cleaned[STATUS_CODE_HEADER_FIELD], HeaderValue::from(201));
        assert_eq!(
            extract_proxy_headers(&as_proxy_headers(source.clone())),
            source
        );

        let mut target = as_proxy_headers(source.clone());
        extend_with_proxy_headers(&mut target, source);
        assert_eq!(target.get_all("zz-spider-r--set-cookie").iter().count(), 4);
    }

    #[test]
    fn runtime_safety_malformed_continuations_are_skipped() {
        let entries = vec![
            (None, HeaderValue::from_static("orphan")),
            (
                Some(HeaderName::from_static("set-cookie")),
                HeaderValue::from_static("a=1"),
            ),
            (None, HeaderValue::from_static("b=2")),
        ];
        let mut builder = WorkerProxyHeaderBuilder::new();
        builder.extend(entries.clone());
        builder.extend([(None, HeaderValue::from_static("another-orphan"))]);
        let built = builder.build();
        assert_eq!(built.len(), 2);
        let mut target = HeaderMap::new();
        extend_with_proxy_headers(&mut target, entries);
        extend_with_proxy_headers(&mut target, [(None, HeaderValue::from_static("orphan"))]);
        assert_eq!(built, target);
    }

    #[test]
    fn runtime_safety_fallible_prefix_parsing() {
        for suffix in ["", "bad name", "bad\nname", "bad:name", "é"] {
            assert!(strip_prefix(format!("{WORKER_PROXY_HEADER_PREFIX}{suffix}")).is_none());
            if !suffix.is_empty() {
                assert!(set_prefix(suffix).is_none());
            }
        }
        assert!(set_prefix("a".repeat(65536)).is_none());
        assert!(strip_prefix("content-type").is_none());
        assert_eq!(
            strip_prefix("zz-spider-r--content-type"),
            Some(reqwest::header::CONTENT_TYPE)
        );
        let mut source = HeaderMap::new();
        source.insert(
            WORKER_PROXY_HEADER_PREFIX,
            HeaderValue::from_static("ignored"),
        );
        assert!(extract_proxy_headers(&source).is_empty());
        assert_eq!(
            split_proxy_headers(source.iter().map(|(k, v)| (k.clone(), v.clone())))
                .original
                .len(),
            1
        );
    }

    #[test]
    fn can_build_a_map() {
        let mut builder = WorkerProxyHeaderBuilder::new();
        builder.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str("value").unwrap(),
        );

        builder.set_status_code(404);
        let map = builder.build();

        assert_eq!(
            map.get(set_prefix(&reqwest::header::USER_AGENT).unwrap())
                .unwrap(),
            HeaderValue::from_str("value").unwrap()
        );

        assert_eq!(
            map.get(PROXIED_ORIGINAL_STATUS).unwrap(),
            HeaderValue::from(404)
        );
    }

    #[test]
    fn can_build_and_clean_a_map() {
        let mut builder = WorkerProxyHeaderBuilder::new();
        builder.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str("value").unwrap(),
        );

        builder.set_status_code(404);
        let map = builder.build();
        let cleaned = extract_proxy_headers(&map);

        assert_eq!(
            cleaned.get(reqwest::header::USER_AGENT).unwrap(),
            HeaderValue::from_str("value").unwrap()
        );

        assert_eq!(
            cleaned.get(STATUS_CODE_HEADER_FIELD).unwrap(),
            HeaderValue::from(404)
        );
    }
}
