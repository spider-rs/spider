//! ETag / conditional-request cache for bandwidth-efficient re-crawls.
//!
//! Feature-gated behind `etag_cache`. Stores `ETag` and `Last-Modified`
//! response headers per URL so subsequent requests can send
//! `If-None-Match` / `If-Modified-Since` and receive lightweight 304
//! responses instead of full bodies.
//!
//! All operations are lock-free via `DashMap`. LRU eviction keeps memory
//! bounded.

#[cfg(feature = "etag_cache")]
mod inner {
    use crate::compact_str::CompactString;
    use dashmap::DashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Maximum cached entries before LRU eviction.
    const MAX_ENTRIES: usize = 50_000;

    /// Stored conditional-request validators for a URL.
    #[derive(Clone, Debug)]
    pub struct ConditionalHeaders {
        /// The `ETag` value from the response, if any.
        pub etag: Option<CompactString>,
        /// The `Last-Modified` value from the response, if any.
        pub last_modified: Option<CompactString>,
        /// Monotonic counter for LRU eviction.
        access: u64,
    }

    /// Cache of conditional-request validators keyed by URL.
    ///
    /// Thread-safe via `DashMap`. No async — callers can use it from any context.
    pub struct ETagCache {
        entries: DashMap<CompactString, ConditionalHeaders>,
        /// Monotonically increasing counter for LRU.
        access_counter: AtomicU64,
    }

    impl ETagCache {
        /// Create a new empty cache.
        pub fn new() -> Self {
            Self {
                entries: DashMap::with_capacity(256),
                access_counter: AtomicU64::new(0),
            }
        }

        /// Store validators from a response. Call after receiving a 200 response
        /// that includes `ETag` or `Last-Modified` headers.
        ///
        /// If neither header is present, no entry is created.
        pub fn store(&self, url: &str, etag: Option<&str>, last_modified: Option<&str>) {
            if etag.is_none() && last_modified.is_none() {
                return;
            }

            let counter = self.access_counter.fetch_add(1, Ordering::Relaxed);
            let key = CompactString::new(url);

            let headers = ConditionalHeaders {
                etag: etag.map(CompactString::new),
                last_modified: last_modified.map(CompactString::new),
                access: counter,
            };

            if let Some(mut existing) = self.entries.get_mut(&key) {
                *existing = headers;
            } else {
                self.maybe_evict();
                self.entries.insert(key, headers);
            }
        }

        /// Retrieve stored validators for a URL, if any.
        ///
        /// Returns `(Option<etag>, Option<last_modified>)`.
        pub fn get(&self, url: &str) -> Option<(Option<CompactString>, Option<CompactString>)> {
            let key = CompactString::new(url);
            self.entries.get_mut(&key).map(|mut entry| {
                entry.access = self.access_counter.fetch_add(1, Ordering::Relaxed);
                (entry.etag.clone(), entry.last_modified.clone())
            })
        }

        /// Build conditional request headers for a URL.
        ///
        /// Returns a vec of `(header_name, header_value)` pairs to add to the
        /// request. Empty if no validators are cached for this URL.
        pub fn conditional_headers(&self, url: &str) -> Vec<(&'static str, CompactString)> {
            let mut headers = Vec::with_capacity(2);

            if let Some((etag, last_modified)) = self.get(url) {
                if let Some(etag) = etag {
                    headers.push(("if-none-match", etag));
                }
                if let Some(lm) = last_modified {
                    headers.push(("if-modified-since", lm));
                }
            }

            headers
        }

        /// Number of cached entries.
        pub fn len(&self) -> usize {
            self.entries.len()
        }

        /// Whether the cache is empty.
        pub fn is_empty(&self) -> bool {
            self.entries.is_empty()
        }

        /// Remove a specific URL's validators.
        pub fn remove(&self, url: &str) {
            let key = CompactString::new(url);
            self.entries.remove(&key);
        }

        /// Clear all cached entries.
        pub fn clear(&self) {
            self.entries.clear();
        }

        /// Evict the least-recently-used entry if over capacity.
        fn maybe_evict(&self) {
            if self.entries.len() < MAX_ENTRIES {
                return;
            }

            let mut oldest_key: Option<CompactString> = None;
            let mut oldest_access = u64::MAX;

            for entry in self.entries.iter() {
                if entry.value().access < oldest_access {
                    oldest_access = entry.value().access;
                    oldest_key = Some(entry.key().clone());
                }
            }

            if let Some(key) = oldest_key {
                self.entries.remove(&key);
            }
        }
    }

    impl Default for ETagCache {
        fn default() -> Self {
            Self::new()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn store_and_retrieve_etag() {
            let cache = ETagCache::new();
            cache.store("https://example.com", Some("\"abc123\""), None);

            let (etag, lm) = cache.get("https://example.com").unwrap();
            assert_eq!(etag.as_deref(), Some("\"abc123\""));
            assert!(lm.is_none());
        }

        #[test]
        fn store_and_retrieve_last_modified() {
            let cache = ETagCache::new();
            cache.store(
                "https://example.com",
                None,
                Some("Thu, 01 Jan 2026 00:00:00 GMT"),
            );

            let (etag, lm) = cache.get("https://example.com").unwrap();
            assert!(etag.is_none());
            assert_eq!(lm.as_deref(), Some("Thu, 01 Jan 2026 00:00:00 GMT"));
        }

        #[test]
        fn store_both_validators() {
            let cache = ETagCache::new();
            cache.store(
                "https://example.com",
                Some("\"v2\""),
                Some("Fri, 20 Mar 2026 12:00:00 GMT"),
            );

            let (etag, lm) = cache.get("https://example.com").unwrap();
            assert_eq!(etag.as_deref(), Some("\"v2\""));
            assert_eq!(lm.as_deref(), Some("Fri, 20 Mar 2026 12:00:00 GMT"));
        }

        #[test]
        fn no_entry_when_neither_header_present() {
            let cache = ETagCache::new();
            cache.store("https://example.com", None, None);
            assert!(cache.get("https://example.com").is_none());
            assert!(cache.is_empty());
        }

        #[test]
        fn conditional_headers_builds_correctly() {
            let cache = ETagCache::new();
            cache.store(
                "https://example.com",
                Some("\"abc\""),
                Some("Thu, 01 Jan 2026 00:00:00 GMT"),
            );

            let headers = cache.conditional_headers("https://example.com");
            assert_eq!(headers.len(), 2);
            assert_eq!(headers[0].0, "if-none-match");
            assert_eq!(headers[0].1.as_str(), "\"abc\"");
            assert_eq!(headers[1].0, "if-modified-since");
        }

        #[test]
        fn conditional_headers_empty_for_unknown_url() {
            let cache = ETagCache::new();
            let headers = cache.conditional_headers("https://unknown.com");
            assert!(headers.is_empty());
        }

        #[test]
        fn overwrite_existing_entry() {
            let cache = ETagCache::new();
            cache.store("https://example.com", Some("\"v1\""), None);
            cache.store("https://example.com", Some("\"v2\""), None);

            let (etag, _) = cache.get("https://example.com").unwrap();
            assert_eq!(etag.as_deref(), Some("\"v2\""));
            assert_eq!(cache.len(), 1);
        }

        #[test]
        fn remove_entry() {
            let cache = ETagCache::new();
            cache.store("https://example.com", Some("\"v1\""), None);
            cache.remove("https://example.com");
            assert!(cache.get("https://example.com").is_none());
        }

        #[test]
        fn clear_all() {
            let cache = ETagCache::new();
            cache.store("https://a.com", Some("\"1\""), None);
            cache.store("https://b.com", Some("\"2\""), None);
            cache.clear();
            assert!(cache.is_empty());
        }

        #[test]
        fn eviction_at_capacity() {
            let cache = ETagCache::new();
            for i in 0..=MAX_ENTRIES {
                cache.store(
                    &format!("https://domain-{i}.com"),
                    Some(&format!("\"etag-{i}\"")),
                    None,
                );
            }
            assert!(cache.len() <= MAX_ENTRIES);
        }

        #[test]
        fn concurrent_access_no_panic() {
            use std::sync::Arc;
            let cache = Arc::new(ETagCache::new());

            let handles: Vec<_> = (0..8)
                .map(|t| {
                    let cache = cache.clone();
                    std::thread::spawn(move || {
                        for i in 0..100 {
                            let url = format!("https://t{t}-{i}.com");
                            cache.store(&url, Some(&format!("\"e{i}\"")), None);
                            let _ = cache.get(&url);
                            let _ = cache.conditional_headers(&url);
                        }
                    })
                })
                .collect();

            for h in handles {
                h.join().unwrap();
            }
            assert!(cache.len() > 0);
        }
    }
}

#[cfg(feature = "etag_cache")]
pub use inner::{ConditionalHeaders, ETagCache};
