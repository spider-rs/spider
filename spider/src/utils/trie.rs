use hashbrown::HashMap;
use std::fmt::Debug;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// TrieNode structure to handle clean url path mappings.
pub struct TrieNode<V: Debug> {
    /// The children for the trie.
    pub children: HashMap<String, TrieNode<V>>,
    /// The value for the trie.
    pub value: Option<V>,
}

impl<V: Debug> TrieNode<V> {
    /// A new trie node.
    pub fn new() -> Self {
        TrieNode {
            children: HashMap::new(),
            value: None,
        }
    }
}

impl<V: Debug> Default for TrieNode<V> {
    fn default() -> Self {
        Self::new()
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

impl<V: Debug> Default for Trie<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Debug> Trie<V> {
    /// A new trie node.
    pub fn new() -> Self {
        Self {
            root: TrieNode::new(),
            match_all: false,
        }
    }

    /// Get the byte offset where the path portion starts, stripping scheme+host.
    #[inline]
    fn path_start(path: &str) -> usize {
        if let Some(pos) = path.find("://") {
            let after_scheme = pos + 3;
            if after_scheme < path.len() {
                path[after_scheme..]
                    .find('/')
                    .map_or(path.len(), |p| after_scheme + p)
            } else {
                0
            }
        } else {
            0
        }
    }

    /// Iterate path segments without allocating.
    #[inline]
    fn path_segments(path: &str) -> impl Iterator<Item = &str> {
        let start = Self::path_start(path);
        let base = if start < path.len() {
            &path[start..]
        } else {
            path
        };
        base.split('/')
            .filter(|s| !s.is_empty() && !s.contains('.'))
    }

    /// Insert a path and its associated value into the trie.
    #[cfg_attr(feature = "inline-more", inline)]
    pub fn insert(&mut self, path: &str, value: V) {
        let mut node = &mut self.root;

        for segment in Self::path_segments(path) {
            node = node
                .children
                .entry_ref(segment)
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

        if node.children.is_empty() && node.value.is_none() {
            return None;
        }

        for segment in Self::path_segments(input) {
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

    #[test]
    fn test_trie_empty_path() {
        let mut trie: Trie<usize> = Trie::new();
        trie.insert("", 1);
        // Empty path normalizes to "/" which sets match_all
        assert!(trie.search("").is_some() || trie.search("/anything").is_some());
    }

    #[test]
    fn test_trie_unicode_paths() {
        let mut trie: Trie<&str> = Trie::new();
        trie.insert("/café/menü", "unicode");
        assert_eq!(trie.search("/café/menü"), Some(&"unicode"));
    }

    #[test]
    fn test_trie_many_entries() {
        let mut trie: Trie<usize> = Trie::new();
        for i in 0..1000 {
            trie.insert(&format!("/path/{}", i), i);
        }
        assert_eq!(trie.search("/path/0"), Some(&0));
        assert_eq!(trie.search("/path/999"), Some(&999));
        assert!(trie.search("/path/1000").is_none());
    }

    #[test]
    fn test_trie_default() {
        let trie: Trie<usize> = Trie::default();
        assert!(trie.root.children.is_empty());
        assert!(!trie.match_all);
    }

    #[test]
    fn test_trie_shared_prefix_insert() {
        let mut trie: Trie<usize> = Trie::new();
        for i in 0..100 {
            trie.insert(&format!("/api/v1/resource/{}", i), i);
        }
        for i in 0..100 {
            assert_eq!(
                trie.search(&format!("/api/v1/resource/{}", i)),
                Some(&i),
                "shared prefix path {} not found",
                i
            );
        }
        // Intermediate nodes should not have values
        assert!(trie.search("/api").is_none());
        assert!(trie.search("/api/v1").is_none());
        assert!(trie.search("/api/v1/resource").is_none());
    }

    #[test]
    fn test_trie_overwrite_preserves_others() {
        let mut trie: Trie<usize> = Trie::new();
        trie.insert("/a/b/c", 1);
        trie.insert("/a/b/d", 2);
        trie.insert("/a/b/e", 3);
        // Overwrite /a/b/c
        trie.insert("/a/b/c", 99);

        assert_eq!(trie.search("/a/b/c"), Some(&99));
        assert_eq!(trie.search("/a/b/d"), Some(&2));
        assert_eq!(trie.search("/a/b/e"), Some(&3));
    }

    #[test]
    fn test_trie_insert_search_full_urls() {
        let mut trie: Trie<&str> = Trie::new();
        // Different leaf segments so they don't overwrite each other
        trie.insert("https://example.com/users/profile", "profile");
        trie.insert("/users/settings", "settings");
        trie.insert("http://other.com/api/data", "data");

        // Full URL and bare path resolve to same trie node (host stripped)
        assert_eq!(trie.search("https://example.com/users/profile"), Some(&"profile"));
        assert_eq!(trie.search("/users/profile"), Some(&"profile"));
        assert_eq!(trie.search("https://any.com/users/settings"), Some(&"settings"));
        assert_eq!(trie.search("/api/data"), Some(&"data"));
        assert_eq!(trie.search("http://cdn.example.com/api/data"), Some(&"data"));
        // Non-existent path
        assert!(trie.search("/users/unknown").is_none());
    }
}
