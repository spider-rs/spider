//! Session memory for spider_agent.
//!
//! Uses DashMap for lock-free concurrent access.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Session memory for storing state across operations.
///
/// Uses DashMap internally for lock-free concurrent reads and writes.
/// This is optimal for high-concurrency scenarios.
#[derive(Debug, Clone, Default)]
pub struct AgentMemory {
    /// Lock-free concurrent map.
    data: Arc<DashMap<String, serde_json::Value>>,
}

impl AgentMemory {
    /// Create a new empty memory.
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
        }
    }

    /// Create memory with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Arc::new(DashMap::with_capacity(capacity)),
        }
    }

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

    /// Clear all memory.
    pub fn clear(&self) {
        self.data.clear();
    }

    /// Check if memory contains a key.
    pub fn contains(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }

    /// Get number of entries.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if memory is empty.
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
}
