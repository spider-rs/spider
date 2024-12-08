/// adblock patterns
pub mod adblock_patterns;
/// amazon blockers
pub mod amazon_blockers;
/// linkedin blockers
pub mod linkedin_blockers;
/// netflix blockers
pub mod netflix_blockers;
/// tiktok blockers
pub mod tiktok_blockers;
/// upwork blockers
pub mod upwork_blockers;
/// x blockers
pub mod x_blockers;

// Trie node for ignore.
#[derive(Default)]
pub(crate) struct TrieNode {
    children: hashbrown::HashMap<char, TrieNode>,
    is_end_of_word: bool,
}

/// Basic Ignore trie.
pub(crate) struct Trie {
    root: TrieNode,
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
