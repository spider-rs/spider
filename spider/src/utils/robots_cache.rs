#![cfg(feature = "robots_cache")]

use case_insensitive_string::compact_str::CompactString;
use dashmap::DashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::Client;

/// Maximum number of cached entries before eviction on overflow.
const MAX_ENTRIES: usize = 10_000;

/// A cached robots.txt entry.
struct RobotsCacheEntry {
    rules_text: String,
    fetched_at: Instant,
    ttl: Duration,
}

impl RobotsCacheEntry {
    fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() < self.ttl
    }
}

/// Global cross-crawl robots.txt cache.
fn global_cache() -> &'static DashMap<CompactString, RobotsCacheEntry> {
    static CACHE: OnceLock<DashMap<CompactString, RobotsCacheEntry>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

/// Retrieve the robots.txt text for a domain, using the global cache.
///
/// Returns the cached text if fresh, otherwise fetches from
/// `https://{domain}/robots.txt`, caches the result, and returns it.
/// Returns `None` if the fetch fails or times out.
///
/// Stores raw text (not a parsed parser) since the robot file parser is not
/// `Send`/`Sync` friendly for a global cache. The caller parses after retrieval.
pub async fn get_or_fetch(domain: &str, client: &Client, ttl: Duration) -> Option<String> {
    let key = CompactString::new(domain);
    let cache = global_cache();

    // Check for a fresh cached entry.
    if let Some(entry) = cache.get(&key) {
        if entry.is_fresh() {
            return Some(entry.rules_text.clone());
        }
    }

    // Fetch robots.txt.
    let url = format!("https://{}/robots.txt", domain);
    let text = fetch_robots_text(&url, client).await?;

    // Evict overflow before inserting.
    if cache.len() >= MAX_ENTRIES {
        evict_expired();
        // If still over capacity after eviction, remove oldest entries.
        if cache.len() >= MAX_ENTRIES {
            // Remove roughly 10% of entries to make room.
            let to_remove = MAX_ENTRIES / 10;
            let keys_to_remove: Vec<CompactString> = cache
                .iter()
                .take(to_remove)
                .map(|entry| entry.key().clone())
                .collect();
            for k in keys_to_remove {
                cache.remove(&k);
            }
        }
    }

    cache.insert(
        key,
        RobotsCacheEntry {
            rules_text: text.clone(),
            fetched_at: Instant::now(),
            ttl,
        },
    );

    Some(text)
}

/// Batch prefetch robots.txt for multiple domains concurrently.
///
/// Fetches all domains in parallel using a `JoinSet`. Failures are silently
/// ignored — the next call to [`get_or_fetch`] will retry.
pub async fn prefetch(domains: &[&str], client: &Client, ttl: Duration) {
    let mut set = tokio::task::JoinSet::new();

    for domain in domains {
        let client = client.clone();
        let ttl = ttl;
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
    global_cache().remove(&key);
}

/// Remove all expired entries from the global cache.
pub fn evict_expired() {
    let cache = global_cache();
    cache.retain(|_, entry| entry.is_fresh());
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

    tokio::time::timeout(Duration::from_secs(10), response.text())
        .await
        .ok()?
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_entry_freshness() {
        let entry = RobotsCacheEntry {
            rules_text: "User-agent: *\nDisallow: /private".to_string(),
            fetched_at: Instant::now(),
            ttl: Duration::from_secs(60),
        };
        assert!(entry.is_fresh());
    }

    #[test]
    fn test_cache_entry_expired() {
        let entry = RobotsCacheEntry {
            rules_text: "User-agent: *\nDisallow:".to_string(),
            fetched_at: Instant::now(),
            ttl: Duration::from_secs(0), // Zero TTL means immediately expired.
        };
        // A zero-TTL entry with any elapsed time should be stale.
        assert!(!entry.is_fresh());
    }

    #[test]
    fn test_invalidate_nonexistent() {
        // Should not panic when invalidating a domain that was never cached.
        invalidate("nonexistent-robots-test.example.com");
    }

    #[test]
    fn test_evict_expired_removes_stale() {
        // Insert an already-expired entry, then evict.
        let cache = global_cache();
        let key = CompactString::new("evict-test.example.com");
        cache.insert(
            key.clone(),
            RobotsCacheEntry {
                rules_text: String::new(),
                fetched_at: Instant::now(),
                ttl: Duration::from_secs(0), // immediately expired
            },
        );
        // Small sleep to ensure elapsed > 0.
        std::thread::sleep(Duration::from_millis(1));
        evict_expired();
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn test_global_cache_singleton() {
        let c1 = global_cache();
        let c2 = global_cache();
        // Both should point to the same DashMap (same OnceLock).
        assert!(std::ptr::eq(c1, c2));
    }

    #[test]
    fn test_manual_cache_insert_and_read() {
        let cache = global_cache();
        let key = CompactString::new("test-manual.example.com");
        cache.insert(
            key.clone(),
            RobotsCacheEntry {
                rules_text: "User-agent: *\nAllow: /".to_string(),
                fetched_at: Instant::now(),
                ttl: Duration::from_secs(300),
            },
        );
        let is_fresh = cache.get(&key).map(|e| e.is_fresh()).unwrap_or(false);
        assert!(is_fresh);
        // Clean up.
        cache.remove(&key);
    }
}
