/// adblock patterns
pub mod adblock_patterns;
/// amazon blockers
pub mod amazon_blockers;
/// ebay blockers
pub mod ebay_blockers;
/// glassdoor blockers
pub mod glassdoor_blockers;
/// interception manager
pub mod intercept_manager;
/// linkedin blockers
pub mod linkedin_blockers;
/// medium blockers
pub mod medium_blockers;
/// netflix blockers
pub mod netflix_blockers;
/// nytimes blockers
pub mod nytimes_blockers;
/// script blockers
pub mod scripts;
/// tiktok blockers
pub mod tiktok_blockers;
/// upwork blockers
pub mod upwork_blockers;
/// wikipedia blockers
pub mod wikipedia_blockers;
/// x blockers
pub mod x_blockers;
/// xhr blockers
pub mod xhr;

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
