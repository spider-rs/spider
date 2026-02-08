use itertools::{Either, Itertools};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::iter::FromIterator;
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
fn set_prefix(name: impl AsRef<str>) -> HeaderName {
    let key_value = name.as_ref();
    let mut new_value = String::with_capacity(WORKER_PROXY_HEADER_PREFIX.len() + key_value.len());
    new_value.push_str(WORKER_PROXY_HEADER_PREFIX);
    new_value.push_str(key_value);

    // Check not necessary, we are valid in 100% of the time.
    match HeaderName::try_from(new_value) {
        Ok(h) => h,
        _ => HeaderName::from_static(""),
    }
}

/// A helper function to strip [WORKER_PROXY_HEADER_PREFIX] of the [name].
/// Returns None if there is no prefix.
pub fn strip_prefix(name: impl AsRef<str>) -> Option<HeaderName> {
    name.as_ref().strip_prefix(WORKER_PROXY_HEADER_PREFIX).map(
        |stripped| match HeaderName::from_str(stripped) {
            Ok(s) => s,
            _ => HeaderName::from_static(""),
        },
    )
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
            target.insert(set_prefix(&k), v);
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
        for value in iter.into_iter() {
            self.headers.push((
                value.0.expect("expected a header name, but got None"),
                value.1,
            ))
        }
    }
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
    target.extend(
        proxied_headers
            .into_iter()
            .map(|(key, value)| (key.map(|key_value| set_prefix(&key_value)), value)),
    )
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
    HeaderMap::from_iter(
        src.into_iter()
            .filter_map(|(key, value)| Some((strip_prefix(key)?, value.clone()))),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        extract_proxy_headers, set_prefix, WorkerProxyHeaderBuilder, PROXIED_ORIGINAL_STATUS,
        STATUS_CODE_HEADER_FIELD,
    };
    use reqwest::header::HeaderValue;

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
            map.get(set_prefix(&reqwest::header::USER_AGENT)).unwrap(),
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
