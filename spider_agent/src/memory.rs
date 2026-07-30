//! Session memory for spider_agent.
//!
//! Uses DashMap for lock-free concurrent access.
//!
//! # Features
//! - **Key-Value Store**: Lock-free concurrent storage for arbitrary JSON values
//! - **URL History**: Track visited URLs for navigation context
//! - **Action History**: Record actions taken for debugging and context
//! - **Extraction History**: Accumulate extracted data across pages
//!
//! Compatible with spider's AutomationMemory patterns while using
//! DashMap for optimal concurrent performance.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Maximum number of actions to keep in history.
const MAX_ACTION_HISTORY: usize = 50;
/// Maximum number of URLs to keep in history.
const MAX_URL_HISTORY: usize = 100;
/// Maximum number of extractions to keep.
const MAX_EXTRACTIONS: usize = 50;
/// Maximum number of distinct keys in the key-value store.
///
/// A deliberately generous bound: normal sessions stay far below it, so it is
/// invisible in practice. It engages only under pathological key churn (e.g. an
/// agent looping and writing a unique key every round), where it caps memory by
/// evicting one older entry per new key. This is DoS / leak protection, not a
/// normal-use path.
const MAX_DATA_ENTRIES: usize = 10_000;

/// A bounded, insertion-ordered history with no lock of its own.
///
/// Backed by a [`DashMap`] keyed by a monotonic sequence number, so ordering is
/// carried by the key rather than by a `Vec` behind an `RwLock`. Readers take
/// no writer-blocking lock, and a push contends only with the shard it lands in.
///
/// Eviction scans for the lowest live key, which is `O(cap)` — with caps of 50
/// to 100 entries that is a handful of integer comparisons, and it stays correct
/// across interleaved [`clear`](Self::clear) calls, which a head cursor would
/// not.
///
/// # Ordering under concurrency
///
/// Sequence assignment and insertion are two steps, so a reader racing a push
/// may observe the history without the in-flight entry (never out of order, and
/// never a torn value). For an LLM context buffer that is equivalent to the
/// read having happened a moment earlier.
#[derive(Debug)]
struct History<T> {
    /// Live entries keyed by their insertion sequence number.
    entries: DashMap<u64, T>,
    /// Next sequence number to hand out. Wrapping is unreachable in practice.
    next_seq: AtomicU64,
    /// Maximum number of retained entries.
    cap: usize,
}

impl<T> History<T> {
    /// Create an empty history bounded to `cap` entries.
    fn new(cap: usize) -> Self {
        Self {
            entries: DashMap::new(),
            next_seq: AtomicU64::new(0),
            cap,
        }
    }

    /// Create an empty history bounded to `cap`, pre-allocating for it.
    fn with_capacity(cap: usize) -> Self {
        Self {
            entries: DashMap::with_capacity(cap),
            next_seq: AtomicU64::new(0),
            cap,
        }
    }

    /// Append an entry, evicting the oldest ones once the cap is exceeded.
    fn push(&self, value: T) {
        self.entries
            .insert(self.next_seq.fetch_add(1, Ordering::Relaxed), value);

        // Loop rather than evict once: concurrent pushes can overshoot the cap
        // together, and a lost race on `remove` must not leave the history over
        // its bound.
        while self.entries.len() > self.cap {
            // Collect the victim key BEFORE removing so no shard guard is held
            // across the mutation — DashMap would self-deadlock otherwise.
            let victim = self.entries.iter().map(|entry| *entry.key()).min();

            match victim {
                Some(victim) => {
                    self.entries.remove(&victim);
                }
                // Emptied by a concurrent `clear`; nothing left to evict.
                None => break,
            }
        }
    }

    /// Every entry, oldest first.
    fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        let mut items: Vec<(u64, T)> = self
            .entries
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect();

        items.sort_unstable_by_key(|(seq, _)| *seq);
        items.into_iter().map(|(_, value)| value).collect()
    }

    /// The last `n` entries, **most recent first**.
    fn recent(&self, n: usize) -> Vec<T>
    where
        T: Clone,
    {
        let mut items: Vec<(u64, T)> = self
            .entries
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect();

        items.sort_unstable_by(|(a, _), (b, _)| b.cmp(a));
        items.into_iter().take(n).map(|(_, value)| value).collect()
    }

    /// Whether any entry matches `needle`.
    ///
    /// Borrowed so a `History<String>` can be probed with a `&str` — no
    /// allocation on the lookup path.
    fn contains<Q>(&self, needle: &Q) -> bool
    where
        T: std::borrow::Borrow<Q>,
        Q: PartialEq + ?Sized,
    {
        self.entries
            .iter()
            .any(|entry| entry.value().borrow() == needle)
    }

    /// Whether the history holds no entries.
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop every entry. The sequence counter keeps advancing, so entries added
    /// afterwards still sort after anything a concurrent reader already saw.
    fn clear(&self) {
        self.entries.clear();
    }
}

/// Session memory for storing state across operations.
///
/// Uses DashMap internally for lock-free concurrent reads and writes.
/// This is optimal for high-concurrency scenarios.
///
/// # Example
/// ```
/// use spider_agent::AgentMemory;
///
/// let memory = AgentMemory::new();
///
/// // Key-value storage
/// memory.set("user_id", serde_json::json!("12345"));
///
/// // URL tracking
/// memory.add_visited_url("https://example.com");
///
/// // Action history
/// memory.add_action("Searched for 'rust frameworks'");
///
/// // Extraction history
/// memory.add_extraction(serde_json::json!({"title": "Example"}));
///
/// // Generate context for LLM
/// let context = memory.to_context_string();
/// ```
#[derive(Debug, Clone)]
pub struct AgentMemory {
    /// Lock-free concurrent key-value store.
    data: Arc<DashMap<String, serde_json::Value>>,
    /// History of visited URLs (most recent last).
    visited_urls: Arc<History<String>>,
    /// Brief summary of recent actions (most recent last).
    action_history: Arc<History<String>>,
    /// History of extracted data from pages (most recent last).
    extractions: Arc<History<serde_json::Value>>,
}

impl Default for AgentMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentMemory {
    /// Create a new empty memory.
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            visited_urls: Arc::new(History::new(MAX_URL_HISTORY)),
            action_history: Arc::new(History::new(MAX_ACTION_HISTORY)),
            extractions: Arc::new(History::new(MAX_EXTRACTIONS)),
        }
    }

    /// Create memory with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Arc::new(DashMap::with_capacity(capacity)),
            visited_urls: Arc::new(History::with_capacity(MAX_URL_HISTORY)),
            action_history: Arc::new(History::with_capacity(MAX_ACTION_HISTORY)),
            extractions: Arc::new(History::with_capacity(MAX_EXTRACTIONS)),
        }
    }

    // ========== Key-Value Store ==========

    /// Get a value from memory.
    ///
    /// Returns a clone of the value to avoid holding refs across await points.
    pub fn get(&self, key: &str) -> Option<serde_json::Value> {
        self.data.get(key).map(|v| v.value().clone())
    }

    /// Set a value in memory.
    ///
    /// The number of distinct keys is bounded by [`MAX_DATA_ENTRIES`]. Updating
    /// an existing key never grows the store; once the cap is reached, adding a
    /// *new* key evicts one arbitrary older entry (the just-set key is always
    /// kept). Below the cap this is a plain insert with no behavior change.
    pub fn set(&self, key: impl Into<String>, value: serde_json::Value) {
        let key = key.into();
        let is_new = !self.data.contains_key(&key);
        self.data.insert(key.clone(), value);
        if is_new && self.data.len() > MAX_DATA_ENTRIES {
            // Collect a victim key BEFORE removing so no shard guard is held
            // across the mutation — DashMap would self-deadlock otherwise.
            let victim = self
                .data
                .iter()
                .find(|entry| entry.key() != &key)
                .map(|entry| entry.key().clone());
            if let Some(victim) = victim {
                self.data.remove(&victim);
            }
        }
    }

    /// Remove a value from memory.
    pub fn remove(&self, key: &str) -> Option<serde_json::Value> {
        self.data.remove(key).map(|(_, v)| v)
    }

    /// Clear all key-value data.
    pub fn clear(&self) {
        self.data.clear();
    }

    /// Check if memory contains a key.
    pub fn contains(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }

    /// Get number of key-value entries.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if key-value store is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Get a typed value from memory.
    pub fn get_as<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Option<T> {
        self.data
            .get(key)
            .and_then(|v| serde_json::from_value(v.value().clone()).ok())
    }

    /// Set a typed value in memory.
    pub fn set_value<T: Serialize>(&self, key: impl Into<String>, value: &T) {
        if let Ok(json) = serde_json::to_value(value) {
            self.data.insert(key.into(), json);
        }
    }

    /// Update a value atomically using a closure.
    ///
    /// The closure receives the current value (if any) and returns the new value.
    pub fn update<F>(&self, key: impl Into<String>, f: F)
    where
        F: FnOnce(Option<&serde_json::Value>) -> serde_json::Value,
    {
        let key = key.into();
        let new_value = f(self.data.get(&key).as_deref());
        self.data.insert(key, new_value);
    }

    /// Get or insert a value.
    pub fn get_or_insert(
        &self,
        key: impl Into<String>,
        default: serde_json::Value,
    ) -> serde_json::Value {
        self.data
            .entry(key.into())
            .or_insert(default)
            .value()
            .clone()
    }

    // ========== URL History ==========

    /// Record a visited URL.
    ///
    /// Keeps the most recent URLs up to the limit.
    pub fn add_visited_url(&self, url: impl Into<String>) {
        self.visited_urls.push(url.into());
    }

    /// Get the list of visited URLs.
    pub fn visited_urls(&self) -> Vec<String> {
        self.visited_urls.to_vec()
    }

    /// Get the last N visited URLs.
    pub fn recent_urls(&self, n: usize) -> Vec<String> {
        self.visited_urls.recent(n)
    }

    /// Check if a URL has been visited.
    pub fn has_visited(&self, url: &str) -> bool {
        self.visited_urls.contains(url)
    }

    /// Clear URL history.
    pub fn clear_urls(&self) {
        self.visited_urls.clear();
    }

    // ========== Action History ==========

    /// Record an action summary.
    ///
    /// Keeps the most recent actions up to the limit.
    pub fn add_action(&self, action: impl Into<String>) {
        self.action_history.push(action.into());
    }

    /// Get the list of actions.
    pub fn action_history(&self) -> Vec<String> {
        self.action_history.to_vec()
    }

    /// Get the last N actions.
    pub fn recent_actions(&self, n: usize) -> Vec<String> {
        self.action_history.recent(n)
    }

    /// Clear action history.
    pub fn clear_actions(&self) {
        self.action_history.clear();
    }

    // ========== Extraction History ==========

    /// Add an extracted value to history.
    ///
    /// Keeps the most recent extractions up to the limit.
    pub fn add_extraction(&self, data: serde_json::Value) {
        self.extractions.push(data);
    }

    /// Get all extractions.
    pub fn extractions(&self) -> Vec<serde_json::Value> {
        self.extractions.to_vec()
    }

    /// Get the last N extractions.
    pub fn recent_extractions(&self, n: usize) -> Vec<serde_json::Value> {
        self.extractions.recent(n)
    }

    /// Clear extraction history.
    pub fn clear_extractions(&self) {
        self.extractions.clear();
    }

    // ========== Bulk Operations ==========

    /// Clear all history (URLs, actions, extractions) but keep key-value store.
    pub fn clear_history(&self) {
        self.visited_urls.clear();
        self.action_history.clear();
        self.extractions.clear();
    }

    /// Clear everything including key-value store and all history.
    pub fn clear_all(&self) {
        self.data.clear();
        self.visited_urls.clear();
        self.action_history.clear();
        self.extractions.clear();
    }

    /// Check if all memory is empty (store + all history).
    pub fn is_all_empty(&self) -> bool {
        self.data.is_empty()
            && self.visited_urls.is_empty()
            && self.action_history.is_empty()
            && self.extractions.is_empty()
    }

    // ========== Context Generation ==========

    /// Generate a context string for inclusion in LLM prompts.
    ///
    /// This provides the LLM with session context including:
    /// - Key-value store contents
    /// - Recent URLs visited
    /// - Recent actions taken
    /// - Recent extractions
    pub fn to_context_string(&self) -> String {
        if self.is_all_empty() {
            return String::new();
        }

        let mut parts = Vec::new();

        // Key-value store
        if !self.data.is_empty() {
            let store: std::collections::HashMap<_, _> = self
                .data
                .iter()
                .map(|r| (r.key().clone(), r.value().clone()))
                .collect();
            if let Ok(json) = serde_json::to_string_pretty(&store) {
                parts.push(format!("## Memory Store\n```json\n{}\n```", json));
            }
        }

        // Recent URLs. `recent` yields newest-first; reverse back to
        // chronological order for the numbered list.
        let recent = self.visited_urls.recent(10);
        if !recent.is_empty() {
            let url_list: String = recent
                .iter()
                .rev()
                .enumerate()
                .map(|(i, u)| format!("{}. {}", i + 1, u))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!(
                "## Recent URLs (last {})\n{}",
                recent.len(),
                url_list
            ));
        }

        // Recent extractions
        let recent = self.extractions.recent(5);
        if !recent.is_empty() {
            let json_strs: Vec<_> = recent
                .iter()
                .rev()
                .filter_map(|v| serde_json::to_string(v).ok())
                .collect();
            parts.push(format!(
                "## Recent Extractions (last {})\n{}",
                json_strs.len(),
                json_strs.join("\n")
            ));
        }

        // Recent actions
        let recent = self.action_history.recent(10);
        if !recent.is_empty() {
            let action_list: String = recent
                .iter()
                .rev()
                .enumerate()
                .map(|(i, a)| format!("{}. {}", i + 1, a))
                .collect::<Vec<_>>()
                .join("\n");
            parts.push(format!(
                "## Recent Actions (last {})\n{}",
                recent.len(),
                action_list
            ));
        }

        parts.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_basic() {
        let memory = AgentMemory::new();

        memory.set("key1", serde_json::json!("value1"));
        memory.set("key2", serde_json::json!(42));

        assert_eq!(memory.get("key1"), Some(serde_json::json!("value1")));
        assert_eq!(memory.get("key2"), Some(serde_json::json!(42)));
        assert_eq!(memory.get("key3"), None);
        assert_eq!(memory.len(), 2);
    }

    #[test]
    fn data_store_is_bounded_and_keeps_recent_key() {
        let memory = AgentMemory::new();
        // Overflow the cap with unique keys (pathological-churn case).
        for i in 0..(MAX_DATA_ENTRIES + 100) {
            memory.set(format!("k{i}"), serde_json::json!(i));
        }
        assert!(memory.len() <= MAX_DATA_ENTRIES);
        // The most recently inserted key is always retained.
        let last = MAX_DATA_ENTRIES + 99;
        assert_eq!(
            memory.get(&format!("k{last}")),
            Some(serde_json::json!(last))
        );
    }

    #[test]
    fn data_store_updates_do_not_count_against_cap() {
        let memory = AgentMemory::new();
        for _ in 0..(MAX_DATA_ENTRIES * 2) {
            memory.set("same", serde_json::json!("v"));
        }
        assert_eq!(memory.len(), 1);
    }

    #[test]
    fn test_memory_typed() {
        let memory = AgentMemory::new();

        memory.set_value("name", &"Alice".to_string());
        memory.set_value("age", &30u32);

        assert_eq!(memory.get_as::<String>("name"), Some("Alice".to_string()));
        assert_eq!(memory.get_as::<u32>("age"), Some(30));
    }

    #[test]
    fn test_memory_clear() {
        let memory = AgentMemory::new();

        memory.set("key1", serde_json::json!("value1"));
        memory.set("key2", serde_json::json!("value2"));

        assert_eq!(memory.len(), 2);

        memory.clear();

        assert!(memory.is_empty());
    }

    #[test]
    fn test_memory_update() {
        let memory = AgentMemory::new();

        memory.set("counter", serde_json::json!(0));

        memory.update("counter", |v| {
            let current = v.and_then(|v| v.as_i64()).unwrap_or(0);
            serde_json::json!(current + 1)
        });

        assert_eq!(memory.get("counter"), Some(serde_json::json!(1)));
    }

    #[test]
    fn test_memory_get_or_insert() {
        let memory = AgentMemory::new();

        let value = memory.get_or_insert("key", serde_json::json!("default"));
        assert_eq!(value, serde_json::json!("default"));

        // Should return existing value
        memory.set("key", serde_json::json!("updated"));
        let value = memory.get_or_insert("key", serde_json::json!("other"));
        assert_eq!(value, serde_json::json!("updated"));
    }

    #[test]
    fn test_memory_concurrent_clone() {
        let memory = AgentMemory::new();
        let memory2 = memory.clone();

        memory.set("key", serde_json::json!("value"));

        // Clone shares the same underlying data
        assert_eq!(memory2.get("key"), Some(serde_json::json!("value")));
    }

    #[test]
    fn test_memory_url_history() {
        let memory = AgentMemory::new();

        memory.add_visited_url("https://example.com");
        memory.add_visited_url("https://example.com/page1");
        memory.add_visited_url("https://example.com/page2");

        assert!(memory.has_visited("https://example.com"));
        assert!(!memory.has_visited("https://other.com"));

        let recent = memory.recent_urls(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0], "https://example.com/page2");
        assert_eq!(recent[1], "https://example.com/page1");

        let all = memory.visited_urls();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_memory_action_history() {
        let memory = AgentMemory::new();

        memory.add_action("Searched for 'rust'");
        memory.add_action("Clicked search button");
        memory.add_action("Extracted results");

        let recent = memory.recent_actions(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0], "Extracted results");
        assert_eq!(recent[1], "Clicked search button");
    }

    #[test]
    fn test_memory_extractions() {
        let memory = AgentMemory::new();

        memory.add_extraction(serde_json::json!({"title": "Page 1"}));
        memory.add_extraction(serde_json::json!({"title": "Page 2"}));

        let extractions = memory.extractions();
        assert_eq!(extractions.len(), 2);

        let recent = memory.recent_extractions(1);
        assert_eq!(recent[0]["title"], "Page 2");
    }

    #[test]
    fn test_memory_clear_all() {
        let memory = AgentMemory::new();

        memory.set("key", serde_json::json!("value"));
        memory.add_visited_url("https://example.com");
        memory.add_action("Test action");
        memory.add_extraction(serde_json::json!({"data": "test"}));

        assert!(!memory.is_all_empty());

        memory.clear_all();

        assert!(memory.is_all_empty());
    }

    #[test]
    fn test_memory_context_string() {
        let memory = AgentMemory::new();

        memory.set("user_id", serde_json::json!("123"));
        memory.add_visited_url("https://example.com");
        memory.add_action("Logged in");

        let context = memory.to_context_string();

        assert!(context.contains("Memory Store"));
        assert!(context.contains("user_id"));
        assert!(context.contains("Recent URLs"));
        assert!(context.contains("example.com"));
        assert!(context.contains("Recent Actions"));
        assert!(context.contains("Logged in"));
    }

    #[test]
    fn history_is_bounded_and_keeps_insertion_order() {
        let memory = AgentMemory::new();

        for i in 0..(MAX_URL_HISTORY + 25) {
            memory.add_visited_url(format!("https://example.com/{i}"));
        }

        let urls = memory.visited_urls();
        assert_eq!(urls.len(), MAX_URL_HISTORY);
        // Oldest entries evicted, order preserved oldest-first.
        assert_eq!(urls[0], "https://example.com/25");
        assert_eq!(
            urls[MAX_URL_HISTORY - 1],
            format!("https://example.com/{}", MAX_URL_HISTORY + 24)
        );
        // The evicted prefix is really gone; the retained tail is still found.
        assert!(!memory.has_visited("https://example.com/0"));
        assert!(memory.has_visited("https://example.com/25"));
    }

    #[test]
    fn history_recent_returns_newest_first() {
        let memory = AgentMemory::new();

        memory.add_action("first");
        memory.add_action("second");
        memory.add_action("third");

        assert_eq!(memory.recent_actions(2), vec!["third", "second"]);
        assert_eq!(memory.action_history(), vec!["first", "second", "third"]);
    }

    #[test]
    fn history_clear_then_push_keeps_ordering() {
        let memory = AgentMemory::new();

        memory.add_action("stale");
        memory.clear_actions();
        assert!(memory.action_history().is_empty());

        // Sequence numbers keep advancing past a clear, so post-clear entries
        // still sort in insertion order rather than colliding with old keys.
        memory.add_action("fresh-1");
        memory.add_action("fresh-2");
        assert_eq!(memory.action_history(), vec!["fresh-1", "fresh-2"]);
    }

    #[test]
    fn history_stays_bounded_under_concurrent_pushes() {
        let memory = AgentMemory::new();
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let memory = memory.clone();
                std::thread::spawn(move || {
                    for i in 0..200 {
                        memory.add_visited_url(format!("https://example.com/{t}/{i}"));
                    }
                })
            })
            .collect();

        for handle in threads {
            handle.join().expect("push thread panicked");
        }

        // 1600 concurrent pushes must not leave the history over its bound.
        assert_eq!(memory.visited_urls().len(), MAX_URL_HISTORY);
    }
}
