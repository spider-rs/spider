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
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Maximum number of actions to keep in history.
const MAX_ACTION_HISTORY: usize = 50;
/// Maximum number of URLs to keep in history.
const MAX_URL_HISTORY: usize = 100;
/// Maximum number of extractions to keep.
const MAX_EXTRACTIONS: usize = 50;

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
#[derive(Debug, Clone, Default)]
pub struct AgentMemory {
    /// Lock-free concurrent key-value store.
    data: Arc<DashMap<String, serde_json::Value>>,
    /// History of visited URLs (most recent last).
    visited_urls: Arc<RwLock<Vec<String>>>,
    /// Brief summary of recent actions (most recent last).
    action_history: Arc<RwLock<Vec<String>>>,
    /// History of extracted data from pages (most recent last).
    extractions: Arc<RwLock<Vec<serde_json::Value>>>,
}

impl AgentMemory {
    /// Create a new empty memory.
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
            visited_urls: Arc::new(RwLock::new(Vec::new())),
            action_history: Arc::new(RwLock::new(Vec::new())),
            extractions: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create memory with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Arc::new(DashMap::with_capacity(capacity)),
            visited_urls: Arc::new(RwLock::new(Vec::with_capacity(MAX_URL_HISTORY))),
            action_history: Arc::new(RwLock::new(Vec::with_capacity(MAX_ACTION_HISTORY))),
            extractions: Arc::new(RwLock::new(Vec::with_capacity(MAX_EXTRACTIONS))),
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
    pub fn set(&self, key: impl Into<String>, value: serde_json::Value) {
        self.data.insert(key.into(), value);
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
        let mut urls = self.visited_urls.write();
        urls.push(url.into());
        if urls.len() > MAX_URL_HISTORY {
            urls.remove(0);
        }
    }

    /// Get the list of visited URLs.
    pub fn visited_urls(&self) -> Vec<String> {
        self.visited_urls.read().clone()
    }

    /// Get the last N visited URLs.
    pub fn recent_urls(&self, n: usize) -> Vec<String> {
        let urls = self.visited_urls.read();
        urls.iter().rev().take(n).cloned().collect()
    }

    /// Check if a URL has been visited.
    pub fn has_visited(&self, url: &str) -> bool {
        self.visited_urls.read().iter().any(|u| u == url)
    }

    /// Clear URL history.
    pub fn clear_urls(&self) {
        self.visited_urls.write().clear();
    }

    // ========== Action History ==========

    /// Record an action summary.
    ///
    /// Keeps the most recent actions up to the limit.
    pub fn add_action(&self, action: impl Into<String>) {
        let mut actions = self.action_history.write();
        actions.push(action.into());
        if actions.len() > MAX_ACTION_HISTORY {
            actions.remove(0);
        }
    }

    /// Get the list of actions.
    pub fn action_history(&self) -> Vec<String> {
        self.action_history.read().clone()
    }

    /// Get the last N actions.
    pub fn recent_actions(&self, n: usize) -> Vec<String> {
        let actions = self.action_history.read();
        actions.iter().rev().take(n).cloned().collect()
    }

    /// Clear action history.
    pub fn clear_actions(&self) {
        self.action_history.write().clear();
    }

    // ========== Extraction History ==========

    /// Add an extracted value to history.
    ///
    /// Keeps the most recent extractions up to the limit.
    pub fn add_extraction(&self, data: serde_json::Value) {
        let mut extractions = self.extractions.write();
        extractions.push(data);
        if extractions.len() > MAX_EXTRACTIONS {
            extractions.remove(0);
        }
    }

    /// Get all extractions.
    pub fn extractions(&self) -> Vec<serde_json::Value> {
        self.extractions.read().clone()
    }

    /// Get the last N extractions.
    pub fn recent_extractions(&self, n: usize) -> Vec<serde_json::Value> {
        let extractions = self.extractions.read();
        extractions.iter().rev().take(n).cloned().collect()
    }

    /// Clear extraction history.
    pub fn clear_extractions(&self) {
        self.extractions.write().clear();
    }

    // ========== Bulk Operations ==========

    /// Clear all history (URLs, actions, extractions) but keep key-value store.
    pub fn clear_history(&self) {
        self.visited_urls.write().clear();
        self.action_history.write().clear();
        self.extractions.write().clear();
    }

    /// Clear everything including key-value store and all history.
    pub fn clear_all(&self) {
        self.data.clear();
        self.visited_urls.write().clear();
        self.action_history.write().clear();
        self.extractions.write().clear();
    }

    /// Check if all memory is empty (store + all history).
    pub fn is_all_empty(&self) -> bool {
        self.data.is_empty()
            && self.visited_urls.read().is_empty()
            && self.action_history.read().is_empty()
            && self.extractions.read().is_empty()
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

        // Recent URLs
        let urls = self.visited_urls.read();
        if !urls.is_empty() {
            let recent: Vec<_> = urls.iter().rev().take(10).collect();
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
        drop(urls);

        // Recent extractions
        let extractions = self.extractions.read();
        if !extractions.is_empty() {
            let recent: Vec<_> = extractions.iter().rev().take(5).collect();
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
        drop(extractions);

        // Recent actions
        let actions = self.action_history.read();
        if !actions.is_empty() {
            let recent: Vec<_> = actions.iter().rev().take(10).collect();
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
        drop(actions);

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
}
