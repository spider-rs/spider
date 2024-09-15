use hashbrown::HashMap;
use std::fmt::Debug;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// TrieNode structure to handle clean url path mappings.
pub struct TrieNode<V: std::fmt::Debug> {
    /// The children for the trie.
    pub children: HashMap<String, TrieNode<V>>,
    /// The value for the trie.
    pub value: Option<V>,
}

impl<V: std::fmt::Debug> TrieNode<V> {
    /// A new trie node.
    pub fn new() -> Self {
        TrieNode {
            children: HashMap::new(),
            value: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Trie value.
pub struct Trie<V: Debug> {
    /// A new trie node.
    pub root: TrieNode<V>,
    /// Contains a match all segment to default to.
    pub match_all: bool,
}

impl<V: Debug> Trie<V> {
    /// A new trie node.
    pub fn new() -> Self {
        Self {
            root: TrieNode::new(),
            match_all: false,
        }
    }

    /// Normalize a url. This will perform a match against paths across all domains.
    fn normalize_path(path: &str) -> String {
        let start_pos = if let Some(pos) = path.find("://") {
            if pos + 3 < path.len() {
                path[pos + 3..]
                    .find('/')
                    .map_or(path.len(), |p| pos + 3 + p)
            } else {
                0
            }
        } else {
            0
        };

        let base_path = if start_pos < path.len() {
            &path[start_pos..]
        } else {
            path
        };

        let normalized_path = base_path
            .split('/')
            .filter(|segment| !segment.is_empty() && !segment.contains('.'))
            .collect::<Vec<_>>()
            .join("/");

        string_concat!("/", normalized_path)
    }

    /// Insert a path and its associated value into the trie.
    #[cfg_attr(feature = "inline-more", inline)]
    pub fn insert(&mut self, path: &str, value: V) {
        let normalized_path = Self::normalize_path(path);
        let mut node = &mut self.root;

        let segments: Vec<&str> = normalized_path
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        for segment in segments {
            node = node
                .children
                .entry(segment.to_string())
                .or_insert_with(TrieNode::new);
        }

        if path == "/" {
            self.match_all = true;
        }

        node.value = Some(value);
    }

    /// Search for a path in the trie.
    #[cfg_attr(feature = "inline-more", inline)]
    pub fn search(&self, input: &str) -> Option<&V> {
        let mut node = &self.root;

        if node.children.is_empty() {
            return None;
        }

        let normalized_path = Self::normalize_path(input);

        for segment in normalized_path.split('/').filter(|s| !s.is_empty()) {
            if let Some(child) = node.children.get(segment) {
                node = child;
            } else if !self.match_all {
                return None;
            }
        }

        node.value.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trie_node_new() {
        let node: TrieNode<usize> = TrieNode::new();
        assert!(node.children.is_empty());
        assert!(node.value.is_none());
    }

    #[test]
    fn test_trie_new() {
        let trie: Trie<usize> = Trie::new();
        assert!(trie.root.children.is_empty());
        assert!(trie.root.value.is_none());
    }

    #[test]
    fn test_insert_and_search() {
        let mut trie: Trie<usize> = Trie::new();
        trie.insert("/path/to/node", 42);
        trie.insert("https://mywebsite/path/to/node", 22);

        assert_eq!(trie.search("https://mywebsite/path/to/node"), Some(&22));
        assert_eq!(trie.search("/path/to/node"), Some(&22));
        assert_eq!(trie.search("/path"), None);
        assert_eq!(trie.search("/path/to"), None);
        assert_eq!(trie.search("/path/to/node/extra"), None);

        // insert match all context
        trie.insert("/", 11);
        assert_eq!(trie.search("/random"), Some(&11));
    }

    #[test]
    fn test_insert_multiple_nodes() {
        let mut trie: Trie<usize> = Trie::new();
        trie.insert("/path/to/node1", 1);
        trie.insert("/path/to/node2", 2);
        trie.insert("/path/to/node3", 3);

        assert_eq!(trie.search("/path/to/node1"), Some(&1));
        assert_eq!(trie.search("/path/to/node2"), Some(&2));
        assert_eq!(trie.search("/path/to/node3"), Some(&3));
    }

    #[test]
    fn test_insert_overwrite() {
        let mut trie: Trie<usize> = Trie::new();
        trie.insert("/path/to/node", 42);
        trie.insert("/path/to/node", 84);

        assert_eq!(trie.search("/path/to/node"), Some(&84));
    }

    #[test]
    fn test_search_nonexistent_path() {
        let mut trie: Trie<usize> = Trie::new();
        trie.insert("/path/to/node", 42);

        assert!(trie.search("/nonexistent").is_none());
        assert!(trie.search("/path/to/wrongnode").is_none());
    }
}
