//! Memory operations for agentic automation sessions.
//!
//! Provides a key-value store and history tracking that persists across
//! automation rounds, enabling the LLM to maintain context and state.

use std::collections::HashMap;

/// Memory operation requested by the LLM.
///
/// These operations allow the model to persist data across automation rounds
/// without requiring external storage.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum MemoryOperation {
    /// Store a value in memory.
    Set {
        /// The key to store under.
        key: String,
        /// The value to store (any JSON value).
        value: serde_json::Value,
    },
    /// Delete a value from memory.
    Delete {
        /// The key to delete.
        key: String,
    },
    /// Clear all stored values.
    Clear,
}

/// In-memory storage for agentic automation sessions.
///
/// This provides a key-value store and history tracking that persists across
/// automation rounds, enabling the LLM to maintain context and state without
/// relying on external storage.
///
/// # Features
/// - **Key-Value Store**: Store and retrieve arbitrary JSON values by key
/// - **Extraction History**: Accumulate extracted data across pages
/// - **URL History**: Track visited URLs for navigation context
/// - **Action Summary**: Brief history of executed actions
///
/// # Example
/// ```rust
/// use spider_agent_types::AutomationMemory;
///
/// let mut memory = AutomationMemory::default();
/// memory.set("user_logged_in", serde_json::json!(true));
/// memory.set("cart_items", serde_json::json!(["item1", "item2"]));
///
/// // Memory is serialized and included in LLM context each round
/// let context = memory.to_context_string();
/// ```
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AutomationMemory {
    /// Key-value store for persistent data across rounds.
    #[serde(default)]
    pub store: HashMap<String, serde_json::Value>,
    /// History of extracted data from pages (most recent last).
    #[serde(default)]
    pub extractions: Vec<serde_json::Value>,
    /// History of visited URLs (most recent last).
    #[serde(default)]
    pub visited_urls: Vec<String>,
    /// Brief summary of recent actions (most recent last, max 50).
    #[serde(default)]
    pub action_history: Vec<String>,
}

impl AutomationMemory {
    const LEVEL_ATTEMPTS_KEY: &'static str = "_level_attempts";

    /// Create a new empty memory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a value by key.
    pub fn set(&mut self, key: impl Into<String>, value: serde_json::Value) {
        self.store.insert(key.into(), value);
    }

    /// Get a value by key.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.store.get(key)
    }

    /// Remove a value by key.
    pub fn remove(&mut self, key: &str) -> Option<serde_json::Value> {
        self.store.remove(key)
    }

    /// Check if a key exists.
    pub fn contains(&self, key: &str) -> bool {
        self.store.contains_key(key)
    }

    /// Clear all stored data.
    pub fn clear_store(&mut self) {
        self.store.clear();
    }

    /// Add an extracted value to history.
    pub fn add_extraction(&mut self, data: serde_json::Value) {
        self.extractions.push(data);
    }

    /// Record a visited URL.
    pub fn add_visited_url(&mut self, url: impl Into<String>) {
        self.visited_urls.push(url.into());
    }

    /// Record an action summary (keeps max 50 entries).
    pub fn add_action(&mut self, action: impl Into<String>) {
        self.action_history.push(action.into());
        // Keep only the last 50 actions to avoid unbounded growth
        if self.action_history.len() > 50 {
            self.action_history.remove(0);
        }
    }

    /// Clear all history (extractions, URLs, actions) but keep the store.
    pub fn clear_history(&mut self) {
        self.extractions.clear();
        self.visited_urls.clear();
        self.action_history.clear();
    }

    /// Clear everything.
    pub fn clear_all(&mut self) {
        self.store.clear();
        self.extractions.clear();
        self.visited_urls.clear();
        self.action_history.clear();
    }

    /// Check if memory is empty.
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
            && self.extractions.is_empty()
            && self.visited_urls.is_empty()
            && self.action_history.is_empty()
    }

    /// Generate a context string for inclusion in LLM prompts.
    pub fn to_context_string(&self) -> String {
        if self.is_empty() {
            return String::new();
        }

        let mut parts = Vec::new();

        if !self.store.is_empty() {
            if let Ok(json) = serde_json::to_string_pretty(&self.store) {
                parts.push(format!("## Memory Store\n```json\n{}\n```", json));
            }
        }

        if !self.visited_urls.is_empty() {
            let recent: Vec<_> = self.visited_urls.iter().rev().take(10).collect();
            parts.push(format!(
                "## Recent URLs (last {})\n{}",
                recent.len(),
                recent
                    .iter()
                    .rev()
                    .enumerate()
                    .map(|(i, u)| format!("{}. {}", i + 1, u))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        if !self.extractions.is_empty() {
            let recent: Vec<_> = self.extractions.iter().rev().take(5).collect();
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

        if !self.action_history.is_empty() {
            let recent: Vec<_> = self.action_history.iter().rev().take(10).collect();
            parts.push(format!(
                "## Recent Actions (last {})\n{}",
                recent.len(),
                recent
                    .iter()
                    .rev()
                    .enumerate()
                    .map(|(i, a)| format!("{}. {}", i + 1, a))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        parts.join("\n\n")
    }

    /// Apply a memory operation.
    pub fn apply_operation(&mut self, op: &MemoryOperation) {
        match op {
            MemoryOperation::Set { key, value } => {
                self.set(key.clone(), value.clone());
            }
            MemoryOperation::Delete { key } => {
                self.remove(key);
            }
            MemoryOperation::Clear => {
                self.clear_store();
            }
        }
    }

    /// Apply multiple memory operations.
    pub fn apply_operations(&mut self, ops: &[MemoryOperation]) {
        for op in ops {
            self.apply_operation(op);
        }
    }

    /// Increment and return attempt count for a logical level key.
    ///
    /// Stored in `store["_level_attempts"][level_key]`.
    pub fn increment_level_attempt(&mut self, level_key: &str) -> u32 {
        let entry = self
            .store
            .entry(Self::LEVEL_ATTEMPTS_KEY.to_string())
            .or_insert_with(|| serde_json::json!({}));

        if !entry.is_object() {
            *entry = serde_json::json!({});
        }

        let map = entry.as_object_mut().expect("level attempts map must be object");
        let current = map
            .get(level_key)
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .saturating_add(1) as u32;
        map.insert(level_key.to_string(), serde_json::json!(current));
        current
    }

    /// Reset attempt count for a logical level key (e.g., after forced refresh).
    pub fn reset_level_attempt(&mut self, level_key: &str) {
        if let Some(entry) = self.store.get_mut(Self::LEVEL_ATTEMPTS_KEY) {
            if let Some(map) = entry.as_object_mut() {
                map.insert(level_key.to_string(), serde_json::json!(0));
            }
        }
    }

    /// Return attempt count for a logical level key.
    pub fn get_level_attempt(&self, level_key: &str) -> u32 {
        self.store
            .get(Self::LEVEL_ATTEMPTS_KEY)
            .and_then(|v| v.as_object())
            .and_then(|m| m.get(level_key))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_operations() {
        let mut memory = AutomationMemory::new();
        assert!(memory.is_empty());

        memory.set("key1", serde_json::json!("value1"));
        assert!(!memory.is_empty());
        assert!(memory.contains("key1"));
        assert_eq!(memory.get("key1"), Some(&serde_json::json!("value1")));

        memory.remove("key1");
        assert!(!memory.contains("key1"));
    }

    #[test]
    fn test_memory_history() {
        let mut memory = AutomationMemory::new();

        memory.add_visited_url("https://example.com/page1");
        memory.add_visited_url("https://example.com/page2");
        assert_eq!(memory.visited_urls.len(), 2);

        memory.add_action("Clicked button");
        memory.add_action("Filled form");
        assert_eq!(memory.action_history.len(), 2);

        memory.clear_history();
        assert!(memory.visited_urls.is_empty());
        assert!(memory.action_history.is_empty());
    }

    #[test]
    fn test_apply_operations() {
        let mut memory = AutomationMemory::new();

        let ops = vec![
            MemoryOperation::Set {
                key: "count".to_string(),
                value: serde_json::json!(42),
            },
            MemoryOperation::Set {
                key: "name".to_string(),
                value: serde_json::json!("test"),
            },
        ];

        memory.apply_operations(&ops);
        assert_eq!(memory.get("count"), Some(&serde_json::json!(42)));
        assert_eq!(memory.get("name"), Some(&serde_json::json!("test")));
    }

    #[test]
    fn test_to_context_string() {
        let mut memory = AutomationMemory::new();
        memory.set("user", serde_json::json!("alice"));
        memory.add_visited_url("https://example.com");

        let context = memory.to_context_string();
        assert!(context.contains("Memory Store"));
        assert!(context.contains("alice"));
        assert!(context.contains("Recent URLs"));
        assert!(context.contains("example.com"));
    }

    #[test]
    fn test_level_attempts_increment_and_get() {
        let mut memory = AutomationMemory::new();

        assert_eq!(memory.get_level_attempt("L7:word-search"), 0);
        assert_eq!(memory.increment_level_attempt("L7:word-search"), 1);
        assert_eq!(memory.increment_level_attempt("L7:word-search"), 2);
        assert_eq!(memory.get_level_attempt("L7:word-search"), 2);
        assert_eq!(memory.increment_level_attempt("L2:image-grid"), 1);
        assert_eq!(memory.get_level_attempt("L2:image-grid"), 1);
    }
}
