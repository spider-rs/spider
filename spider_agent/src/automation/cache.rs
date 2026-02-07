//! Smart caching with automatic space management.
//!
//! Features:
//! - Bounded memory usage with configurable limits
//! - LRU eviction with size-aware cleanup
//! - TTL-based expiration
//! - Automatic cleanup on memory pressure

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Size-aware LRU cache with automatic cleanup.
///
/// Manages cache size based on both entry count and estimated memory usage.
#[derive(Debug)]
pub struct SmartCache<V: CacheValue> {
    entries: Arc<RwLock<HashMap<String, CacheEntry<V>>>>,
    /// Maximum number of entries.
    max_entries: usize,
    /// Maximum total size in bytes.
    max_size_bytes: usize,
    /// Current estimated size.
    current_size: Arc<AtomicUsize>,
    /// Default TTL for entries.
    default_ttl: Duration,
    /// Stats tracking.
    stats: Arc<CacheStats>,
}

/// Cache entry with metadata.
#[derive(Debug, Clone)]
pub struct CacheEntry<V> {
    /// The cached value.
    pub value: V,
    /// When this entry was created.
    pub created_at: Instant,
    /// When this entry was last accessed.
    pub last_accessed: Instant,
    /// Estimated size in bytes.
    pub size_bytes: usize,
    /// Time-to-live for this entry.
    pub ttl: Duration,
    /// Access count.
    pub access_count: u32,
}

/// Trait for cache values that can report their size.
pub trait CacheValue: Clone + Send + Sync + 'static {
    /// Estimate the size of this value in bytes.
    fn estimated_size(&self) -> usize;
}

// Implement for common types
impl CacheValue for String {
    fn estimated_size(&self) -> usize {
        self.len() + std::mem::size_of::<String>()
    }
}

impl CacheValue for Vec<u8> {
    fn estimated_size(&self) -> usize {
        self.len() + std::mem::size_of::<Vec<u8>>()
    }
}

impl CacheValue for serde_json::Value {
    fn estimated_size(&self) -> usize {
        // Rough estimate based on JSON serialization
        serde_json::to_string(self).map(|s| s.len()).unwrap_or(100)
            + std::mem::size_of::<serde_json::Value>()
    }
}

/// Cache statistics.
#[derive(Debug, Default)]
pub struct CacheStats {
    hits: AtomicUsize,
    misses: AtomicUsize,
    evictions: AtomicUsize,
    expirations: AtomicUsize,
}

impl CacheStats {
    /// Get hit count.
    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    /// Get miss count.
    pub fn misses(&self) -> usize {
        self.misses.load(Ordering::Relaxed)
    }

    /// Get eviction count.
    pub fn evictions(&self) -> usize {
        self.evictions.load(Ordering::Relaxed)
    }

    /// Get hit rate.
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits() as f64;
        let total = hits + self.misses() as f64;
        if total > 0.0 {
            hits / total
        } else {
            0.0
        }
    }

    /// Reset stats.
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.expirations.store(0, Ordering::Relaxed);
    }
}

impl<V: CacheValue> SmartCache<V> {
    /// Create a new cache with default settings.
    ///
    /// Default: 1000 entries, 50MB max size, 5 minute TTL.
    pub fn new() -> Self {
        Self::with_limits(1000, 50 * 1024 * 1024, Duration::from_secs(300))
    }

    /// Create with custom limits.
    pub fn with_limits(max_entries: usize, max_size_bytes: usize, default_ttl: Duration) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::with_capacity(max_entries.min(10000)))),
            max_entries,
            max_size_bytes,
            current_size: Arc::new(AtomicUsize::new(0)),
            default_ttl,
            stats: Arc::new(CacheStats::default()),
        }
    }

    /// Create a small cache (100 entries, 5MB).
    pub fn small() -> Self {
        Self::with_limits(100, 5 * 1024 * 1024, Duration::from_secs(60))
    }

    /// Create a large cache (10000 entries, 200MB).
    pub fn large() -> Self {
        Self::with_limits(10000, 200 * 1024 * 1024, Duration::from_secs(600))
    }

    /// Get a value from the cache.
    pub async fn get(&self, key: &str) -> Option<V> {
        let mut entries = self.entries.write().await;

        if let Some(entry) = entries.get_mut(key) {
            // Check expiration
            if entry.created_at.elapsed() > entry.ttl {
                let size = entry.size_bytes;
                entries.remove(key);
                self.current_size.fetch_sub(size, Ordering::Relaxed);
                self.stats.expirations.fetch_add(1, Ordering::Relaxed);
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                return None;
            }

            entry.last_accessed = Instant::now();
            entry.access_count += 1;
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
            Some(entry.value.clone())
        } else {
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Set a value in the cache.
    pub async fn set(&self, key: impl Into<String>, value: V) {
        self.set_with_ttl(key, value, self.default_ttl).await;
    }

    /// Set a value with custom TTL.
    pub async fn set_with_ttl(&self, key: impl Into<String>, value: V, ttl: Duration) {
        let key = key.into();
        let size = value.estimated_size() + key.len() + std::mem::size_of::<CacheEntry<V>>();

        // Ensure we have space
        self.ensure_space(size).await;

        let entry = CacheEntry {
            value,
            created_at: Instant::now(),
            last_accessed: Instant::now(),
            size_bytes: size,
            ttl,
            access_count: 1,
        };

        let mut entries = self.entries.write().await;

        // Remove old entry if exists
        if let Some(old) = entries.remove(&key) {
            self.current_size
                .fetch_sub(old.size_bytes, Ordering::Relaxed);
        }

        entries.insert(key, entry);
        self.current_size.fetch_add(size, Ordering::Relaxed);
    }

    /// Remove a value from the cache.
    pub async fn remove(&self, key: &str) -> Option<V> {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.remove(key) {
            self.current_size
                .fetch_sub(entry.size_bytes, Ordering::Relaxed);
            Some(entry.value)
        } else {
            None
        }
    }

    /// Clear the entire cache.
    pub async fn clear(&self) {
        let mut entries = self.entries.write().await;
        entries.clear();
        self.current_size.store(0, Ordering::Relaxed);
    }

    /// Get current entry count.
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }

    /// Check if cache is empty.
    pub async fn is_empty(&self) -> bool {
        self.entries.read().await.is_empty()
    }

    /// Get current size in bytes.
    pub fn size_bytes(&self) -> usize {
        self.current_size.load(Ordering::Relaxed)
    }

    /// Get cache statistics.
    pub fn stats(&self) -> &CacheStats {
        &self.stats
    }

    /// Ensure there's enough space for a new entry.
    async fn ensure_space(&self, needed_bytes: usize) {
        // Check entry count
        let entry_count = self.entries.read().await.len();
        if entry_count >= self.max_entries {
            self.evict_lru().await;
        }

        // Check size limit
        let current = self.current_size.load(Ordering::Relaxed);
        if current + needed_bytes > self.max_size_bytes {
            self.evict_until_space(needed_bytes).await;
        }
    }

    /// Evict the least recently used entry.
    async fn evict_lru(&self) {
        let mut entries = self.entries.write().await;

        if let Some(lru_key) = entries
            .iter()
            .min_by_key(|(_, e)| e.last_accessed)
            .map(|(k, _)| k.clone())
        {
            if let Some(entry) = entries.remove(&lru_key) {
                self.current_size
                    .fetch_sub(entry.size_bytes, Ordering::Relaxed);
                self.stats.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Evict entries until we have enough space.
    async fn evict_until_space(&self, needed_bytes: usize) {
        let target = self.max_size_bytes.saturating_sub(needed_bytes);

        while self.current_size.load(Ordering::Relaxed) > target {
            self.evict_lru().await;

            // Safety: don't loop forever
            if self.entries.read().await.is_empty() {
                break;
            }
        }
    }

    /// Clean up expired entries.
    pub async fn cleanup_expired(&self) {
        let mut entries = self.entries.write().await;
        let now = Instant::now();

        let expired: Vec<String> = entries
            .iter()
            .filter(|(_, e)| now.duration_since(e.created_at) > e.ttl)
            .map(|(k, _)| k.clone())
            .collect();

        for key in expired {
            if let Some(entry) = entries.remove(&key) {
                self.current_size
                    .fetch_sub(entry.size_bytes, Ordering::Relaxed);
                self.stats.expirations.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Start a background cleanup task.
    ///
    /// Runs cleanup every `interval` duration.
    pub fn start_cleanup_task(self: Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                self.cleanup_expired().await;
            }
        })
    }
}

impl<V: CacheValue> Default for SmartCache<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// Specialized cache for HTML content.
pub type HtmlCache = SmartCache<String>;

/// Specialized cache for JSON responses.
pub type JsonCache = SmartCache<serde_json::Value>;

/// Create a bounded HTML cache.
pub fn html_cache(max_pages: usize, max_mb: usize) -> HtmlCache {
    SmartCache::with_limits(max_pages, max_mb * 1024 * 1024, Duration::from_secs(300))
}

/// Create a bounded JSON cache.
pub fn json_cache(max_entries: usize, max_mb: usize) -> JsonCache {
    SmartCache::with_limits(max_entries, max_mb * 1024 * 1024, Duration::from_secs(300))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_smart_cache_basic() {
        let cache: SmartCache<String> = SmartCache::new();

        cache.set("key1", "value1".to_string()).await;
        assert_eq!(cache.get("key1").await, Some("value1".to_string()));
        assert_eq!(cache.get("key2").await, None);
    }

    #[tokio::test]
    async fn test_smart_cache_eviction() {
        let cache: SmartCache<String> =
            SmartCache::with_limits(2, 1024 * 1024, Duration::from_secs(60));

        cache.set("key1", "value1".to_string()).await;
        cache.set("key2", "value2".to_string()).await;
        cache.set("key3", "value3".to_string()).await; // Should evict key1

        assert_eq!(cache.len().await, 2);
        assert_eq!(cache.get("key3").await, Some("value3".to_string()));
    }

    #[tokio::test]
    async fn test_smart_cache_size_limit() {
        // 1KB limit
        let cache: SmartCache<String> = SmartCache::with_limits(100, 1024, Duration::from_secs(60));

        // Add entries until size limit is hit
        for i in 0..50 {
            cache.set(format!("key{}", i), "x".repeat(100)).await;
        }

        // Should have evicted some entries
        assert!(cache.size_bytes() <= 1024 + 200); // Some overhead allowed
    }

    #[tokio::test]
    async fn test_smart_cache_ttl() {
        let cache: SmartCache<String> =
            SmartCache::with_limits(100, 1024 * 1024, Duration::from_millis(50));

        cache.set("key1", "value1".to_string()).await;

        // Should exist immediately
        assert!(cache.get("key1").await.is_some());

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should be expired
        assert!(cache.get("key1").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let cache: SmartCache<String> = SmartCache::new();

        cache.set("key1", "value1".to_string()).await;

        cache.get("key1").await; // Hit
        cache.get("key1").await; // Hit
        cache.get("key2").await; // Miss

        let stats = cache.stats();
        assert_eq!(stats.hits(), 2);
        assert_eq!(stats.misses(), 1);
        assert!((stats.hit_rate() - 0.666).abs() < 0.01);
    }
}
