use case_insensitive_string::compact_str::CompactString;
use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::Client;
use tokio_stream::StreamExt;

/// Maximum number of cached entries.
const MAX_ENTRIES: usize = 10_000;
/// Bound retained text and keys independently of entry count.
const MAX_CACHE_BYTES: usize = 16 * 1024 * 1024;
/// Bound decoded HTTP bytes before charset conversion.
const MAX_BODY_BYTES: usize = 512 * 1024;
const SWEEP_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Budget {
    entries: AtomicUsize,
    bytes: AtomicUsize,
}

/// Reservations travel with entries, including across replacement and removal.
/// This keeps concurrent insertions within both limits without a separate lock.
struct Reservation {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Budget {
    fn reserve(self: &Arc<Self>, bytes: usize) -> Option<Reservation> {
        self.entries
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < MAX_ENTRIES).then_some(n + 1)
            })
            .ok()?;
        if self
            .bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                n.checked_add(bytes)
                    .filter(|total| *total <= MAX_CACHE_BYTES)
            })
            .is_err()
        {
            self.entries.fetch_sub(1, Ordering::Relaxed);
            return None;
        }
        Some(Reservation {
            budget: Arc::clone(self),
            bytes,
        })
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.bytes.fetch_sub(self.bytes, Ordering::Relaxed);
        self.budget.entries.fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct RobotsCache {
    entries: DashMap<CompactString, RobotsCacheEntry>,
    budget: Arc<Budget>,
    sweeping: AtomicBool,
}

impl RobotsCache {
    fn insert(self: &Arc<Self>, key: CompactString, text: &str, ttl: Duration) {
        if ttl.is_zero() {
            return;
        }
        // Boxed strings retain exactly their length; count key capacity too.
        let Some(bytes) = text.len().checked_add(key.capacity()) else {
            return;
        };
        if bytes > MAX_CACHE_BYTES {
            return;
        }
        let reservation = self.budget.reserve(bytes).or_else(|| {
            // Preserve admission under churn. Collect keys before removing so
            // no DashMap read guard is held while acquiring a write guard.
            let to_remove = (self.entries.len() / 10).clamp(1, MAX_ENTRIES / 10);
            let keys: Vec<_> = self
                .entries
                .iter()
                .take(to_remove)
                .map(|entry| entry.key().clone())
                .collect();
            for key in keys {
                self.entries.remove(&key);
            }
            self.budget.reserve(bytes)
        });
        let Some(reservation) = reservation else {
            return;
        };
        self.entries.insert(
            key,
            RobotsCacheEntry {
                rules_text: text.into(),
                fetched_at: Instant::now(),
                ttl,
                _reservation: reservation,
            },
        );
        self.start_sweeper();
    }

    fn evict_expired(&self) {
        self.entries.retain(|_, entry| entry.is_fresh());
    }

    fn start_sweeper(self: &Arc<Self>) {
        if self
            .sweeping
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        // One worker per active cache, independent of the caller's Tokio runtime.
        // It exits when empty; no task or timer is allocated per entry.
        let cache = Arc::clone(self);
        if std::thread::Builder::new()
            .name("spider-robots-cache".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(SWEEP_INTERVAL);
                    cache.evict_expired();
                    if cache.entries.is_empty() {
                        cache.sweeping.store(false, Ordering::Release);
                        // Pair with insertion's CAS so an insertion racing worker exit
                        // either starts a new worker or is serviced by this one.
                        if cache.entries.is_empty()
                            || cache
                                .sweeping
                                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                                .is_err()
                        {
                            break;
                        }
                    }
                }
            })
            .is_err()
        {
            // A failed spawn must not leave cached bodies without maintenance.
            self.sweeping.store(false, Ordering::Release);
            self.entries.clear();
        }
    }
}

/// A cached robots.txt entry.
struct RobotsCacheEntry {
    rules_text: Box<str>,
    _reservation: Reservation,
    fetched_at: Instant,
    ttl: Duration,
}

impl RobotsCacheEntry {
    fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() < self.ttl
    }
}

/// Global cross-crawl robots.txt cache.
fn global_cache() -> &'static Arc<RobotsCache> {
    static CACHE: OnceLock<Arc<RobotsCache>> = OnceLock::new();
    CACHE.get_or_init(|| Arc::new(RobotsCache::default()))
}

/// Retrieve the robots.txt text for a domain, using the global cache.
///
/// Returns the cached text if fresh, otherwise fetches from
/// `https://{domain}/robots.txt`, caches the result, and returns it.
/// Returns `None` if the fetch fails, times out, or exceeds 512 KiB.
/// A maintenance worker reclaims expired entries at one-second intervals.
/// Overflow evicts up to 1,000 entries before retrying admission against the
/// shared 16 MiB and 10,000-entry limits; an uncached result is still returned.
///
/// Stores raw text (not a parsed parser) since the robot file parser is not
/// `Send`/`Sync` friendly for a global cache. The caller parses after retrieval.
pub async fn get_or_fetch(domain: &str, client: &Client, ttl: Duration) -> Option<String> {
    let key = CompactString::new(domain);
    let cache = global_cache();

    // Check for a fresh cached entry.
    if let Some(entry) = cache.entries.get(&key) {
        if entry.is_fresh() {
            return Some(entry.rules_text.to_string());
        }
    }

    // Remove a stale entry even if its refresh subsequently fails.
    cache.entries.remove_if(&key, |_, entry| !entry.is_fresh());

    let url = format!("https://{}/robots.txt", domain);
    let text = fetch_robots_text(&url, client).await?;
    #[cfg(not(target_arch = "wasm32"))]
    cache.insert(key, &text, ttl);
    // Browser targets cannot run the idle maintenance thread. Return fetched
    // text without retaining it rather than leaving expired bodies resident.
    #[cfg(target_arch = "wasm32")]
    let _ = ttl;

    Some(text)
}

/// Batch prefetch robots.txt for multiple domains concurrently.
///
/// Fetches up to 16 domains concurrently using a `JoinSet`. Failures are silently
/// ignored — the next call to [`get_or_fetch`] will retry.
pub async fn prefetch(domains: &[&str], client: &Client, ttl: Duration) {
    let mut set = tokio::task::JoinSet::new();

    for domain in domains {
        if set.len() >= 16 {
            set.join_next().await;
        }
        let client = client.clone();
        let domain = domain.to_string();
        set.spawn(async move {
            get_or_fetch(&domain, &client, ttl).await;
        });
    }

    while set.join_next().await.is_some() {}
}

/// Remove a cached entry for the given domain.
pub fn invalidate(domain: &str) {
    let key = CompactString::new(domain);
    global_cache().entries.remove(&key);
}

/// Remove all expired entries from the global cache.
pub fn evict_expired() {
    let cache = global_cache();
    cache.evict_expired();
}

/// Fetch robots.txt text from the given URL. Returns `None` on any error.
async fn fetch_robots_text(url: &str, client: &Client) -> Option<String> {
    let response = tokio::time::timeout(Duration::from_secs(10), client.get(url).send())
        .await
        .ok()?
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    tokio::time::timeout(Duration::from_secs(10), async move {
        #[cfg(not(target_arch = "wasm32"))]
        let headers = response.headers().clone();
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.ok()?;
            if chunk.len() > MAX_BODY_BYTES - body.len() {
                return None;
            }
            body.extend_from_slice(&chunk);
        }
        // Preserve the HTTP client's charset/BOM decoding after bounded reading.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut bounded_response = http::Response::new(body);
            *bounded_response.headers_mut() = headers;
            crate::client::Response::from(bounded_response)
                .text()
                .await
                .ok()
        }
        #[cfg(target_arch = "wasm32")]
        {
            // Browser Response.text() decodes UTF-8 and strips a leading BOM.
            let body = body.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&body);
            Some(String::from_utf8_lossy(body).into_owned())
        }
    })
    .await
    .ok()?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_bounds_concurrent_admission_and_reclaims_reservations() {
        let budget = Arc::new(Budget::default());
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let budget = &budget;
                scope.spawn(move || {
                    let reservations: Vec<_> = (0..100)
                        .filter_map(|_| budget.reserve(MAX_BODY_BYTES))
                        .collect();
                    assert!(budget.bytes.load(Ordering::Relaxed) <= MAX_CACHE_BYTES);
                    drop(reservations);
                });
            }
        });
        assert_eq!(budget.entries.load(Ordering::Relaxed), 0);
        assert_eq!(budget.bytes.load(Ordering::Relaxed), 0);
        let reservations: Vec<_> = (0..MAX_ENTRIES)
            .map(|_| budget.reserve(0).unwrap())
            .collect();
        assert!(budget.reserve(0).is_none());
        drop(reservations);
        assert!(budget.reserve(MAX_CACHE_BYTES).is_some());
        assert!(budget.reserve(MAX_CACHE_BYTES + 1).is_none());
    }

    #[test]
    fn idle_expiry_releases_body_and_worker_without_runtime() {
        let cache = Arc::new(RobotsCache::default());
        cache.insert(
            "idle.example".into(),
            "User-agent: *",
            Duration::from_millis(1),
        );
        assert_eq!(cache.budget.entries.load(Ordering::Relaxed), 1);
        let weak = Arc::downgrade(&cache);
        drop(cache);
        let deadline = Instant::now() + Duration::from_secs(5);
        while weak.strong_count() != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(weak.strong_count(), 0, "idle worker retained its cache");
    }

    #[test]
    fn idle_worker_restarts_after_new_admission() {
        let cache = Arc::new(RobotsCache::default());
        cache.insert("restart.example".into(), "first", Duration::from_millis(1));
        let deadline = Instant::now() + Duration::from_secs(5);
        while cache.sweeping.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!cache.sweeping.load(Ordering::Acquire));
        assert!(cache.entries.is_empty());
        cache.insert("restart.example".into(), "second", Duration::from_millis(1));
        let weak = Arc::downgrade(&cache);
        drop(cache);
        let deadline = Instant::now() + Duration::from_secs(5);
        while weak.strong_count() != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    fn byte_overflow_keeps_most_cached_domains() {
        let cache = Arc::new(RobotsCache::default());
        let body = "x".repeat(MAX_BODY_BYTES);
        for i in 0..32 {
            cache.insert(
                format!("overflow-{i}.example").into(),
                &body,
                Duration::from_secs(60),
            );
        }
        assert!(cache.entries.len() >= 28);
        assert!(cache.entries.contains_key("overflow-31.example"));
        assert!(cache.budget.bytes.load(Ordering::Relaxed) <= MAX_CACHE_BYTES);
        cache.entries.clear();
    }

    #[test]
    fn replacement_removal_and_zero_ttl_release_budget() {
        let cache = Arc::new(RobotsCache::default());
        let key = CompactString::new("replace.example");
        cache.insert(key.clone(), "first", Duration::from_secs(60));
        cache.insert(key.clone(), "second", Duration::from_secs(60));
        assert_eq!(cache.budget.entries.load(Ordering::Relaxed), 1);
        assert_eq!(
            cache.budget.bytes.load(Ordering::Relaxed),
            key.capacity() + 6
        );
        cache.entries.remove(&key);
        assert_eq!(cache.budget.bytes.load(Ordering::Relaxed), 0);
        cache.insert(key, "uncached", Duration::ZERO);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn concurrent_insertion_keeps_actual_cache_within_budget() {
        let cache = Arc::new(RobotsCache::default());
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let cache = &cache;
                scope.spawn(move || {
                    let body = "x".repeat(MAX_BODY_BYTES);
                    for i in 0..16 {
                        cache.insert(
                            format!("{worker}-{i}.example").into(),
                            &body,
                            Duration::from_secs(60),
                        );
                    }
                });
            }
        });
        let bytes: usize = cache
            .entries
            .iter()
            .map(|entry| entry.rules_text.len() + entry.key().capacity())
            .sum();
        assert!(bytes <= MAX_CACHE_BYTES);
        assert_eq!(bytes, cache.budget.bytes.load(Ordering::Relaxed));
        assert_eq!(
            cache.entries.len(),
            cache.budget.entries.load(Ordering::Relaxed)
        );
        cache.entries.clear();
        assert_eq!(cache.budget.bytes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn fresh_hit_and_failed_refresh_release_stale_entry() {
        let cache = global_cache();
        let domain = "[invalid-robots-cache-test]";
        let key = CompactString::new(domain);
        let client = crate::ClientBuilder::new().no_proxy().build().unwrap();
        #[cfg(feature = "cache_request")]
        let client = reqwest_middleware::ClientBuilder::new(client).build();
        cache.insert(key.clone(), "fresh", Duration::from_secs(60));
        assert_eq!(
            get_or_fetch(domain, &client, Duration::from_secs(60))
                .await
                .as_deref(),
            Some("fresh")
        );
        cache.entries.get_mut(&key).unwrap().ttl = Duration::ZERO;
        assert!(get_or_fetch(domain, &client, Duration::from_secs(60))
            .await
            .is_none());
        assert!(!cache.entries.contains_key(&key));
    }

    async fn fetch_test_body(body: Vec<u8>, headers: &str) -> Option<String> {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let headers = headers.to_string();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request);
            let _ = write!(
                socket,
                "HTTP/1.1 200 OK\r\nConnection: close\r\n{headers}\r\n"
            );
            let _ = socket.write_all(&body);
        });
        let client = crate::ClientBuilder::new().no_proxy().build().unwrap();
        #[cfg(feature = "cache_request")]
        let client = reqwest_middleware::ClientBuilder::new(client).build();
        let result = fetch_robots_text(&format!("http://{addr}/robots.txt"), &client).await;
        server.join().unwrap();
        result
    }

    #[tokio::test]
    async fn streamed_bodies_are_bounded_without_content_length() {
        assert_eq!(
            fetch_test_body(vec![b'x'; MAX_BODY_BYTES], "")
                .await
                .unwrap()
                .len(),
            MAX_BODY_BYTES
        );
        assert!(fetch_test_body(vec![b'x'; MAX_BODY_BYTES + 1], "")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn preserves_response_charset_decoding() {
        assert_eq!(
            fetch_test_body(
                vec![0xe9],
                "Content-Type: text/plain; charset=windows-1252\r\n"
            )
            .await
            .as_deref(),
            Some("é")
        );
    }
}
