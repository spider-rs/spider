use hashbrown::HashMap;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// TrieNode structure to handle clean url path mappings.
pub struct TrieNode<V: std::fmt::Debug> {
    /// The children for the trie.
    pub children: HashMap<String, TrieNode<V>>,
    /// The value for the trie.
    pub value: Option<V>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Trie value.
pub struct Trie<V: std::fmt::Debug> {
    /// The root node.
    pub root: TrieNode<V>,
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

impl<V: std::fmt::Debug> Trie<V> {
    /// A new trie node.
    pub fn new() -> Self {
        Self {
            root: TrieNode::new(),
        }
    }

    /// Insert a path and its associated value into the trie.
    pub fn insert(&mut self, path: &str, value: V) {
        let mut node = &mut self.root;

        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        for segment in segments {
            node = node
                .children
                .entry(segment.to_string())
                .or_insert_with(TrieNode::new);
        }

        node.value = Some(value);
    }

    /// Search for a path in the trie.
    pub fn search(&self, path: &str) -> Option<&V> {
        let mut node = &self.root;

        for segment in path.split('/').filter(|s| !s.is_empty()) {
            if let Some(child) = node.children.get(segment) {
                node = child;
            } else {
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
        trie.insert("https://mywebsite/path/to/node", 42);

        assert_eq!(trie.search("https://mywebsite/path/to/node"), Some(&42));
        assert_eq!(trie.search("/path/to/node"), Some(&42));
        assert_eq!(trie.search("/path"), None);
        assert_eq!(trie.search("/path/to"), None);
        assert_eq!(trie.search("/path/to/node/extra"), None);
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
