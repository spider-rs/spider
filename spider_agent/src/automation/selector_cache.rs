//! Self-healing selector cache for automation.
//!
//! Caches mappings from natural language element descriptions to CSS selectors.
//! When a cached selector fails, it's invalidated and the system re-queries
//! for an updated selector.

use std::collections::HashMap;

/// Self-healing selector cache.
///
/// Stores mappings from natural language descriptions to CSS selectors
/// that successfully matched elements. Supports LRU eviction.
///
/// # Self-Healing Flow
/// 1. User requests action like "click the login button"
/// 2. Cache lookup: if cached selector for "login button" exists, try it
/// 3. If selector works → action succeeds, update cache timestamp
/// 4. If selector fails → invalidate entry, re-query LLM for new selector
/// 5. Store new selector in cache for future use
#[derive(Debug, Clone)]
pub struct SelectorCache {
    /// Maps normalized element descriptions to cached selectors.
    entries: HashMap<String, SelectorCacheEntry>,
    /// Maximum entries before LRU eviction.
    max_entries: usize,
    /// Cache hit count.
    hits: u64,
    /// Cache miss count.
    misses: u64,
}

impl Default for SelectorCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectorCache {
    /// Default maximum entries.
    const DEFAULT_MAX_ENTRIES: usize = 1000;

    /// Create a new selector cache with default capacity.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            max_entries: Self::DEFAULT_MAX_ENTRIES,
            hits: 0,
            misses: 0,
        }
    }

    /// Create with specified capacity.
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(max_entries.min(10000)),
            max_entries,
            hits: 0,
            misses: 0,
        }
    }

    /// Normalize a description key for consistent lookup.
    fn normalize_key(description: &str) -> String {
        description.trim().to_lowercase()
    }

    /// Get current timestamp in milliseconds.
    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Look up a cached selector for an element description.
    ///
    /// Optionally filter by domain for site-specific selectors.
    pub fn get(&mut self, description: &str, domain: Option<&str>) -> Option<&str> {
        let key = Self::normalize_key(description);

        if let Some(entry) = self.entries.get(&key) {
            // Check domain match if specified
            if let Some(d) = domain {
                if let Some(ref cached_domain) = entry.domain {
                    if cached_domain != d {
                        self.misses += 1;
                        return None;
                    }
                }
            }
            self.hits += 1;
            Some(&entry.selector)
        } else {
            self.misses += 1;
            None
        }
    }

    /// Record a successful selector use.
    pub fn record_success(&mut self, description: &str, selector: &str, domain: Option<&str>) {
        let key = Self::normalize_key(description);
        let now_ms = Self::now_ms();

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.success_count = entry.success_count.saturating_add(1);
            entry.last_used_ms = now_ms;
            entry.selector = selector.to_string();
        } else {
            // Evict LRU if at capacity
            if self.entries.len() >= self.max_entries {
                self.evict_lru();
            }
            self.entries.insert(
                key,
                SelectorCacheEntry {
                    selector: selector.to_string(),
                    success_count: 1,
                    failure_count: 0,
                    last_used_ms: now_ms,
                    domain: domain.map(|s| s.to_string()),
                },
            );
        }
    }

    /// Record a selector failure and potentially invalidate.
    ///
    /// After 2 consecutive failures, the entry is removed.
    pub fn record_failure(&mut self, description: &str) {
        let key = Self::normalize_key(description);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.failure_count = entry.failure_count.saturating_add(1);
            // Invalidate after repeated failures
            if entry.failure_count >= 2 {
                self.entries.remove(&key);
            }
        }
    }

    /// Invalidate (remove) a cache entry.
    pub fn invalidate(&mut self, description: &str) {
        let key = Self::normalize_key(description);
        self.entries.remove(&key);
    }

    /// Clear all cache entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.hits = 0;
        self.misses = 0;
    }

    /// Evict the least recently used entry.
    fn evict_lru(&mut self) {
        if let Some(lru_key) = self
            .entries
            .iter()
            .min_by_key(|(_, v)| v.last_used_ms)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&lru_key);
        }
    }

    /// Get cache statistics: (hits, misses, entry_count).
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            entry_count: self.entries.len(),
            hit_rate: if self.hits + self.misses > 0 {
                self.hits as f64 / (self.hits + self.misses) as f64
            } else {
                0.0
            },
        }
    }

    /// Get number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get all cached selectors for a domain.
    pub fn selectors_for_domain(&self, domain: &str) -> Vec<(&str, &str)> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.domain.as_deref() == Some(domain))
            .map(|(desc, entry)| (desc.as_str(), entry.selector.as_str()))
            .collect()
    }

    /// Export cache entries (for persistence).
    pub fn export(&self) -> Vec<SelectorCacheEntry> {
        self.entries.values().cloned().collect()
    }

    /// Import cache entries (from persistence).
    pub fn import(&mut self, entries: impl IntoIterator<Item = (String, SelectorCacheEntry)>) {
        for (key, entry) in entries {
            let normalized = Self::normalize_key(&key);
            if self.entries.len() < self.max_entries {
                self.entries.insert(normalized, entry);
            }
        }
    }
}

/// A single entry in the selector cache.
#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SelectorCacheEntry {
    /// The CSS selector that matched.
    pub selector: String,
    /// Number of successful uses.
    pub success_count: u32,
    /// Number of failures (before invalidation).
    pub failure_count: u32,
    /// Timestamp of last successful use (unix millis).
    pub last_used_ms: u64,
    /// Domain where this selector was discovered.
    pub domain: Option<String>,
}

impl SelectorCacheEntry {
    /// Create a new cache entry.
    pub fn new(selector: impl Into<String>) -> Self {
        Self {
            selector: selector.into(),
            success_count: 0,
            failure_count: 0,
            last_used_ms: 0,
            domain: None,
        }
    }

    /// Set domain.
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Get reliability score (success rate).
    pub fn reliability(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            0.0
        } else {
            self.success_count as f64 / total as f64
        }
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    /// Number of cache hits.
    pub hits: u64,
    /// Number of cache misses.
    pub misses: u64,
    /// Number of entries in cache.
    pub entry_count: usize,
    /// Hit rate (0.0 to 1.0).
    pub hit_rate: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selector_cache_basic() {
        let mut cache = SelectorCache::new();

        // Miss on first lookup
        assert!(cache.get("login button", None).is_none());

        // Record success
        cache.record_success("login button", "button.login", None);

        // Hit on second lookup
        assert_eq!(cache.get("login button", None), Some("button.login"));

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_selector_cache_failure_invalidation() {
        let mut cache = SelectorCache::new();

        cache.record_success("submit button", "button.submit", None);
        assert!(cache.get("submit button", None).is_some());

        // First failure doesn't invalidate
        cache.record_failure("submit button");
        assert!(cache.get("submit button", None).is_some());

        // Second failure invalidates
        cache.record_failure("submit button");
        assert!(cache.get("submit button", None).is_none());
    }

    #[test]
    fn test_selector_cache_domain_filtering() {
        let mut cache = SelectorCache::new();

        cache.record_success("login", "button.login-a", Some("example.com"));
        cache.record_success("login", "button.login-b", Some("other.com"));

        // Domain match
        assert_eq!(
            cache.get("login", Some("example.com")),
            Some("button.login-b") // Last recorded wins
        );

        // No domain filter returns any match
        assert!(cache.get("login", None).is_some());
    }

    #[test]
    fn test_selector_cache_normalization() {
        let mut cache = SelectorCache::new();

        cache.record_success("  Login Button  ", "button.login", None);

        // Different casing and whitespace should match
        assert!(cache.get("login button", None).is_some());
        assert!(cache.get("LOGIN BUTTON", None).is_some());
        assert!(cache.get("  login button  ", None).is_some());
    }

    #[test]
    fn test_selector_cache_lru_eviction() {
        let mut cache = SelectorCache::with_capacity(2);

        cache.record_success("btn1", "sel1", None);
        std::thread::sleep(std::time::Duration::from_millis(10));
        cache.record_success("btn2", "sel2", None);
        std::thread::sleep(std::time::Duration::from_millis(10));

        // This should evict btn1 (oldest)
        cache.record_success("btn3", "sel3", None);

        assert!(cache.get("btn1", None).is_none());
        assert!(cache.get("btn2", None).is_some());
        assert!(cache.get("btn3", None).is_some());
    }

    #[test]
    fn test_cache_entry_reliability() {
        let mut entry = SelectorCacheEntry::new("button.test");
        entry.success_count = 8;
        entry.failure_count = 2;

        assert!((entry.reliability() - 0.8).abs() < 0.001);
    }
}
