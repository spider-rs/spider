/// adblock patterns
pub mod adblock_patterns;
/// Block websites from spider_firewall list
pub mod block_websites;
/// interception manager
pub mod intercept_manager;
/// script blockers
pub mod scripts;
/// xhr blockers
pub mod xhr;

// Trie node for ignore.
#[derive(Default, Debug)]
pub struct TrieNode {
    /// Children for trie.
    pub children: hashbrown::HashMap<char, TrieNode>,
    /// End of word match.
    pub is_end_of_word: bool,
}

/// Basic Ignore trie.
#[derive(Debug)]
pub struct Trie {
    /// The trie node.
    pub root: TrieNode,
}

impl Trie {
    /// Setup a new trie.
    pub fn new() -> Self {
        Trie {
            root: TrieNode::default(),
        }
    }
    // Insert a word into the Trie.
    pub fn insert(&mut self, word: &str) {
        let mut node = &mut self.root;
        for ch in word.chars() {
            node = node.children.entry(ch).or_insert_with(TrieNode::default);
        }
        node.is_end_of_word = true;
    }

    // Check if the Trie contains any prefix of the given string.
    #[inline]
    pub fn contains_prefix(&self, text: &str) -> bool {
        let mut node = &self.root;

        for ch in text.chars() {
            if let Some(next_node) = node.children.get(&ch) {
                node = next_node;
                if node.is_end_of_word {
                    return true;
                }
            } else {
                break;
            }
        }

        false
    }
}

/// Url matches analytics that we want to ignore or trackers.
pub(crate) fn ignore_script_embedded(url: &str) -> bool {
    crate::handler::blockers::scripts::URL_IGNORE_EMBEDED_TRIE.contains_prefix(url)
}

/// Url matches analytics that we want to ignore or trackers.
pub(crate) fn ignore_script_xhr(url: &str) -> bool {
    crate::handler::blockers::xhr::URL_IGNORE_XHR_TRIE.contains_prefix(url)
}

/// Url matches media that we want to ignore.
pub(crate) fn ignore_script_xhr_media(url: &str) -> bool {
    crate::handler::blockers::xhr::URL_IGNORE_XHR_MEDIA_TRIE.contains_prefix(url)
}
