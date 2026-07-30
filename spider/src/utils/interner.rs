use crate::CaseInsensitiveString;
use hashbrown::HashSet;
use std::hash::Hash;

#[cfg(feature = "bloom")]
use crate::utils::bloom::MmapBloom;

#[cfg(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
))]
use std::marker::PhantomData;

#[cfg(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
))]
use string_interner::symbol::SymbolUsize;

#[cfg(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
))]
use string_interner::StringInterner;

#[cfg(feature = "string_interner_buffer_backend")]
type Backend = string_interner::backend::BufferBackend<SymbolUsize>;

#[cfg(all(
    not(feature = "string_interner_buffer_backend"),
    feature = "string_interner_string_backend",
))]
type Backend = string_interner::backend::StringBackend<SymbolUsize>;

#[cfg(all(
    not(feature = "string_interner_buffer_backend"),
    not(feature = "string_interner_string_backend"),
    feature = "string_interner_bucket_backend",
))]
type Backend = string_interner::backend::BucketBackend<SymbolUsize>;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
))]
/// The links visited bucket store.
///
/// # Memory bounding
///
/// The in-memory visited set grows by one interned URL per crawled page. A
/// **bounded** visited set requires one of two features:
///
/// - `disk` — `Website::insert_link` spills to sqlite once the set reaches
///   `LINKS_VISITED_MEMORY_LIMIT`. `ListBucket` stays out of the way entirely in
///   that configuration.
/// - `bloom` — once the set reaches `LINKS_VISITED_MEMORY_LIMIT` the bucket
///   switches to *bloom-only* mode: the set and its interner are released and
///   the mmap-backed bloom filter becomes the sole membership oracle. A bloom
///   filter has **no false negatives**, so an already-visited URL can never be
///   reported as unvisited — crawl termination is preserved. The trade is a
///   small, bounded false-positive rate: a never-visited URL may occasionally be
///   reported as visited and skipped.
///
/// With **neither** feature the visited set is unbounded and a long crawl will
/// grow without limit; a one-shot warning is logged when the limit is first
/// crossed. Entries are never silently dropped, because evicting from the set
/// would produce false negatives and could make the crawler loop forever.
pub struct ListBucket<K = CaseInsensitiveString>
where
    K: Eq + Hash + Clone + AsRef<str>,
{
    pub(crate) links_visited: HashSet<SymbolUsize>,
    pub(crate) interner: StringInterner<Backend>,
    _marker: PhantomData<K>,
    /// mmap-backed bloom filter pre-check for O(1) membership queries.
    #[cfg(feature = "bloom")]
    pub(crate) bloom: MmapBloom,
    /// The in-memory set hit the memory limit and was released — the bloom
    /// filter is now the sole membership oracle. See the type-level docs.
    #[cfg(feature = "bloom")]
    pub(crate) bloom_only: bool,
    /// One-shot latch for the unbounded-growth warning.
    #[cfg(all(not(feature = "bloom"), not(feature = "disk")))]
    pub(crate) limit_warned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(not(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
)))]
/// The links visited bucket store.
///
/// See the interner-backed variant of this type for the full memory-bounding
/// contract: a bounded visited set requires either the `disk` or the `bloom`
/// feature. With neither, the set is unbounded and a one-shot warning is logged
/// when `LINKS_VISITED_MEMORY_LIMIT` is first crossed.
pub struct ListBucket<K = CaseInsensitiveString>
where
    K: Eq + Hash + Clone + AsRef<str>,
{
    pub(crate) links_visited: HashSet<K>,
    /// mmap-backed bloom filter pre-check for O(1) membership queries.
    #[cfg(feature = "bloom")]
    pub(crate) bloom: MmapBloom,
    /// The in-memory set hit the memory limit and was released — the bloom
    /// filter is now the sole membership oracle.
    #[cfg(feature = "bloom")]
    pub(crate) bloom_only: bool,
    /// One-shot latch for the unbounded-growth warning.
    #[cfg(all(not(feature = "bloom"), not(feature = "disk")))]
    pub(crate) limit_warned: bool,
}

#[cfg(not(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
)))]
impl<K> Default for ListBucket<K>
where
    K: Eq + Hash + Clone + AsRef<str>,
{
    fn default() -> Self {
        Self {
            links_visited: HashSet::new(),
            #[cfg(feature = "bloom")]
            bloom: MmapBloom::with_default_capacity(),
            #[cfg(feature = "bloom")]
            bloom_only: false,
            #[cfg(all(not(feature = "bloom"), not(feature = "disk")))]
            limit_warned: false,
        }
    }
}

#[cfg(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
))]
impl<K> Default for ListBucket<K>
where
    K: Eq + Hash + Clone + AsRef<str>,
{
    fn default() -> Self {
        Self {
            links_visited: HashSet::new(),
            interner: StringInterner::new(),
            _marker: PhantomData,
            #[cfg(feature = "bloom")]
            bloom: MmapBloom::with_default_capacity(),
            #[cfg(feature = "bloom")]
            bloom_only: false,
            #[cfg(all(not(feature = "bloom"), not(feature = "disk")))]
            limit_warned: false,
        }
    }
}

impl<K> ListBucket<K>
where
    K: Eq + Hash + Clone + AsRef<str>,
{
    /// New list bucket.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the bucket has fallen back to bloom-only membership because the
    /// in-memory visited set reached `LINKS_VISITED_MEMORY_LIMIT`.
    ///
    /// In this mode [`Self::len`] no longer tracks the number of visited links
    /// and [`Self::get_links`] no longer returns them, but [`Self::contains`]
    /// remains free of false negatives.
    #[cfg(feature = "bloom")]
    #[inline(always)]
    pub fn bloom_only(&self) -> bool {
        self.bloom_only
    }

    /// The configured in-memory visited-link ceiling.
    ///
    /// Reads `crate::website::LINKS_VISITED_MEMORY_LIMIT`, which is parsed once
    /// from the identically named env var with a 15 000 default and falls back to
    /// that default on a malformed value (never panics).
    #[cfg(not(feature = "disk"))]
    #[inline(always)]
    fn memory_limit() -> usize {
        *crate::website::LINKS_VISITED_MEMORY_LIMIT
    }

    /// Release the in-memory visited set and its interner backing storage.
    ///
    /// Does **not** touch the bloom filter — [`Self::clear`] is the full reset.
    #[inline]
    fn clear_in_memory(&mut self) {
        self.links_visited.clear();
        // Reclaim the interner: clearing only `links_visited` leaves every
        // interned URL's bytes allocated in the `StringInterner` for the life of
        // the bucket, so `clear()` would not actually free memory. Reassigning a
        // fresh interner drops that backing storage.
        #[cfg(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        ))]
        {
            self.interner = StringInterner::new();
        }
    }

    /// Enforce the in-memory visited-set bound. Called after every insert.
    ///
    /// Compiles away entirely under `disk`, which bounds the set itself by
    /// spilling to sqlite in `Website::insert_link`; interfering there would
    /// substitute an approximate oracle for an exact one.
    #[inline(always)]
    fn enforce_memory_limit(&mut self) {
        #[cfg(all(feature = "bloom", not(feature = "disk")))]
        {
            if !self.bloom_only && self.links_visited.len() >= Self::memory_limit() {
                // The bloom filter already holds every link inserted so far, so
                // dropping the set cannot introduce a false negative.
                self.bloom_only = true;
                self.clear_in_memory();
            }
        }

        #[cfg(all(not(feature = "bloom"), not(feature = "disk")))]
        {
            if !self.limit_warned && self.links_visited.len() >= Self::memory_limit() {
                self.limit_warned = true;
                log::warn!(
                    "visited-link set passed {} entries and is unbounded: neither the `disk` nor the `bloom` feature is enabled. Enable `bloom` (in-memory, approximate) or `disk` (sqlite spill) to bound it. Entries are not dropped, because evicting them would make visited URLs look unvisited and could prevent the crawl from terminating.",
                    Self::memory_limit()
                );
            }
        }
    }

    /// Add a new link to the bucket.
    #[inline(always)]
    pub fn insert(&mut self, link: K) {
        #[cfg(feature = "bloom")]
        {
            self.bloom.insert(link.as_ref());

            // Bloom-only mode: the set and interner were released, the filter is
            // the sole membership oracle and has just been fed. Nothing else to do.
            if self.bloom_only {
                return;
            }
        }

        #[cfg(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        ))]
        {
            self.links_visited
                .insert(self.interner.get_or_intern(link.as_ref()));
        }

        #[cfg(not(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        )))]
        {
            self.links_visited.insert(link);
        }

        self.enforce_memory_limit();
    }

    /// Remove a link from the visited set so it can be retried.
    ///
    /// Bloom filter membership is intentionally not cleared (bloom filters do
    /// not support removal). A stale bloom bit only forces the HashSet lookup,
    /// which correctly reports absence after this call.
    ///
    /// In bloom-only mode (see the type-level docs) removal is not possible and
    /// this returns `false` without changing anything — the link stays "visited".
    /// That is the safe direction: it can only skip a retry, never re-crawl.
    #[inline(always)]
    pub fn remove(&mut self, link: &K) -> bool {
        #[cfg(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        ))]
        {
            if let Some(symbol) = self.interner.get(link.as_ref()) {
                self.links_visited.remove(&symbol)
            } else {
                false
            }
        }

        #[cfg(not(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        )))]
        {
            self.links_visited.remove(link)
        }
    }

    /// Does the bucket contain the link.
    ///
    /// When the `bloom` feature is enabled, the mmap-backed bloom filter is
    /// checked first.  A negative result is authoritative (no false negatives),
    /// so the HashSet lookup is skipped entirely — this is the fast path for
    /// the vast majority of unseen URLs.
    #[inline(always)]
    pub fn contains(&self, link: &K) -> bool {
        #[cfg(feature = "bloom")]
        {
            // Bloom filter says "definitely not present" → skip HashSet.
            if !self.bloom.contains(link.as_ref()) {
                return false;
            }
            // Bloom-only mode: the set was released, so the filter is
            // authoritative for positives. Bloom filters have no false
            // negatives, so a visited link is never reported as unvisited and
            // the crawl still terminates; a false positive merely skips a page.
            if self.bloom_only {
                return true;
            }
        }

        #[cfg(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        ))]
        {
            if let Some(symbol) = self.interner.get(link.as_ref()) {
                self.links_visited.contains(&symbol)
            } else {
                false
            }
        }

        #[cfg(not(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        )))]
        {
            self.links_visited.contains(link)
        }
    }

    /// The bucket length.
    ///
    /// This is the size of the in-memory set. In bloom-only mode it no longer
    /// tracks the number of visited links (the set was released).
    pub fn len(&self) -> usize {
        self.links_visited.len()
    }

    /// The bucket is empty.
    ///
    /// Reflects the in-memory set only; see [`Self::len`].
    pub fn is_empty(&self) -> bool {
        self.links_visited.is_empty()
    }

    /// Drain the bucket.
    #[cfg(any(
        feature = "string_interner_bucket_backend",
        feature = "string_interner_string_backend",
        feature = "string_interner_buffer_backend",
    ))]
    pub fn drain(&mut self) -> hashbrown::hash_set::Drain<'_, SymbolUsize> {
        self.links_visited.drain()
    }

    #[cfg(not(any(
        feature = "string_interner_bucket_backend",
        feature = "string_interner_string_backend",
        feature = "string_interner_buffer_backend",
    )))]
    /// Drain the bucket.
    pub fn drain(&mut self) -> hashbrown::hash_set::Drain<'_, K> {
        self.links_visited.drain()
    }

    /// Clear the bucket.
    pub fn clear(&mut self) {
        self.clear_in_memory();
        #[cfg(feature = "bloom")]
        {
            self.bloom.clear();
            // Everything is unvisited again, so the (empty) set is authoritative
            // once more — leave bloom-only mode so the bound can re-arm.
            self.bloom_only = false;
        }
        #[cfg(all(not(feature = "bloom"), not(feature = "disk")))]
        {
            self.limit_warned = false;
        }
    }

    /// Get a vector of all the inner values of the links in the bucket.
    ///
    /// In bloom-only mode the in-memory set has been released, so this returns
    /// only the links still held there (none). A bloom filter is not enumerable.
    pub fn get_links(&self) -> HashSet<K>
    where
        K: Hash + Clone + From<String>,
    {
        #[cfg(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        ))]
        {
            self.links_visited
                .iter()
                .filter_map(|symbol| self.interner.resolve(*symbol))
                .map(|s| K::from(s.to_string()))
                .collect()
        }

        #[cfg(not(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        )))]
        {
            self.links_visited.clone()
        }
    }

    /// Extend with current links.
    #[inline(always)]
    pub fn extend_links(&mut self, links: &mut HashSet<K>, msg: HashSet<K>)
    where
        K: Clone,
    {
        #[cfg(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        ))]
        {
            for link in msg {
                // Bloom pre-check: skip HashSet lookup when definitely absent.
                #[cfg(feature = "bloom")]
                {
                    if self.bloom.contains(link.as_ref()) {
                        // Bloom-only mode: the filter is authoritative for
                        // positives — treat as visited.
                        if self.bloom_only {
                            continue;
                        }
                        // Use read-only `get` — no allocation if already interned.
                        if let Some(symbol) = self.interner.get(link.as_ref()) {
                            if self.links_visited.contains(&symbol) {
                                continue;
                            }
                        }
                    }
                }
                #[cfg(not(feature = "bloom"))]
                {
                    // Use read-only `get` — avoids interning strings we'll never visit.
                    if let Some(symbol) = self.interner.get(link.as_ref()) {
                        if self.links_visited.contains(&symbol) {
                            continue;
                        }
                    }
                }
                links.insert(link);
            }
        }

        #[cfg(not(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        )))]
        {
            #[cfg(feature = "bloom")]
            {
                for link in msg {
                    if !self.bloom.contains(link.as_ref()) {
                        links.insert(link);
                    } else if !self.bloom_only && !self.links_visited.contains(&link) {
                        // Bloom-only mode: a bloom positive is authoritative —
                        // treat as visited and skip.
                        links.insert(link);
                    }
                }
            }
            #[cfg(not(feature = "bloom"))]
            {
                // `msg` is owned — iterate by value and move, avoiding `.cloned()`.
                for link in msg {
                    if !self.links_visited.contains(&link) {
                        links.insert(link);
                    }
                }
            }
        }
    }

    /// Extend with new links.
    #[inline(always)]
    pub fn extend_with_new_links(&mut self, links: &mut HashSet<K>, s: K)
    where
        K: Clone,
    {
        // Bloom pre-check: if bloom says "not present", skip the HashSet lookup.
        #[cfg(feature = "bloom")]
        {
            if !self.bloom.contains(s.as_ref()) {
                links.insert(s);
                return;
            }
            // Bloom-only mode: a bloom positive is authoritative — already visited.
            if self.bloom_only {
                return;
            }
        }

        #[cfg(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        ))]
        {
            if let Some(symbol) = self.interner.get(s.as_ref()) {
                if !self.links_visited.contains(&symbol) {
                    links.insert(s);
                }
            } else {
                links.insert(s);
            }
        }

        #[cfg(not(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        )))]
        {
            if !self.links_visited.contains(&s) {
                links.insert(s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_bucket_new() {
        let bucket: ListBucket<CaseInsensitiveString> = ListBucket::new();
        assert!(bucket.is_empty());
        assert_eq!(bucket.len(), 0);
    }

    #[test]
    fn test_list_bucket_insert_contains() {
        let mut bucket = ListBucket::new();
        let link = CaseInsensitiveString::from("https://example.com");
        bucket.insert(link.clone());
        assert!(bucket.contains(&link));
        assert!(!bucket.contains(&CaseInsensitiveString::from("https://other.com")));
    }

    #[test]
    fn test_list_bucket_len_and_is_empty() {
        let mut bucket = ListBucket::new();
        assert!(bucket.is_empty());
        assert_eq!(bucket.len(), 0);

        bucket.insert(CaseInsensitiveString::from("https://a.com"));
        assert!(!bucket.is_empty());
        assert_eq!(bucket.len(), 1);

        bucket.insert(CaseInsensitiveString::from("https://b.com"));
        assert_eq!(bucket.len(), 2);
    }

    #[test]
    fn test_list_bucket_clear() {
        let mut bucket = ListBucket::new();
        bucket.insert(CaseInsensitiveString::from("https://a.com"));
        bucket.insert(CaseInsensitiveString::from("https://b.com"));
        assert_eq!(bucket.len(), 2);

        bucket.clear();
        assert!(bucket.is_empty());
        assert_eq!(bucket.len(), 0);
    }

    #[test]
    fn test_list_bucket_drain() {
        let mut bucket = ListBucket::new();
        bucket.insert(CaseInsensitiveString::from("https://a.com"));
        bucket.insert(CaseInsensitiveString::from("https://b.com"));

        let drained: Vec<_> = bucket.drain().collect();
        assert_eq!(drained.len(), 2);
        assert!(bucket.is_empty());
    }

    #[test]
    fn test_list_bucket_get_links() {
        let mut bucket = ListBucket::new();
        bucket.insert(CaseInsensitiveString::from("https://a.com"));
        bucket.insert(CaseInsensitiveString::from("https://b.com"));

        let links = bucket.get_links();
        assert_eq!(links.len(), 2);
        assert!(links.contains(&CaseInsensitiveString::from("https://a.com")));
        assert!(links.contains(&CaseInsensitiveString::from("https://b.com")));
    }

    #[test]
    fn test_list_bucket_extend_links() {
        let mut bucket = ListBucket::new();
        bucket.insert(CaseInsensitiveString::from("https://visited.com"));

        let mut links = HashSet::new();
        let mut msg = HashSet::new();
        msg.insert(CaseInsensitiveString::from("https://visited.com"));
        msg.insert(CaseInsensitiveString::from("https://new.com"));

        bucket.extend_links(&mut links, msg);
        assert_eq!(links.len(), 1);
        assert!(links.contains(&CaseInsensitiveString::from("https://new.com")));
    }

    #[test]
    fn test_list_bucket_extend_with_new_links() {
        let mut bucket = ListBucket::new();
        bucket.insert(CaseInsensitiveString::from("https://visited.com"));

        let mut links = HashSet::new();

        bucket.extend_with_new_links(
            &mut links,
            CaseInsensitiveString::from("https://visited.com"),
        );
        assert!(links.is_empty());

        bucket.extend_with_new_links(&mut links, CaseInsensitiveString::from("https://new.com"));
        assert_eq!(links.len(), 1);
        assert!(links.contains(&CaseInsensitiveString::from("https://new.com")));
    }

    /// The core soundness property of bloom-only mode: once the in-memory set is
    /// released, **every** previously inserted link must still report `true`.
    /// A single false negative would make the crawler re-visit a page and, in the
    /// worst case, never terminate.
    #[test]
    #[cfg(all(feature = "bloom", not(feature = "disk")))]
    fn test_list_bucket_bloom_only_has_no_false_negatives() {
        let limit = *crate::website::LINKS_VISITED_MEMORY_LIMIT;
        let total = limit.saturating_add(1000);

        let links: Vec<CaseInsensitiveString> = (0..total)
            .map(|i| CaseInsensitiveString::from(format!("https://example.com/{}", i).as_str()))
            .collect();

        let mut bucket: ListBucket<CaseInsensitiveString> = ListBucket::new();

        for link in &links {
            bucket.insert(link.clone());
        }

        assert!(
            bucket.bloom_only(),
            "bloom-only mode should engage once the {} entry limit is reached",
            limit
        );
        assert!(
            bucket.len() < limit,
            "the in-memory set must stop growing (len = {}, limit = {})",
            bucket.len(),
            limit
        );

        for link in &links {
            assert!(
                bucket.contains(link),
                "false negative after bloom-only fallback for {}",
                link.as_ref()
            );
        }
    }

    /// Normal (under-limit) operation must be completely unaffected: unvisited
    /// URLs still report `false` and the set still holds every entry.
    #[test]
    #[cfg(feature = "bloom")]
    fn test_list_bucket_under_limit_unaffected() {
        let mut bucket: ListBucket<CaseInsensitiveString> = ListBucket::new();

        for i in 0..100 {
            bucket.insert(CaseInsensitiveString::from(
                format!("https://example.com/{}", i).as_str(),
            ));
        }

        assert!(!bucket.bloom_only());
        assert_eq!(bucket.len(), 100);
        assert!(bucket.contains(&CaseInsensitiveString::from("https://example.com/50")));
        assert!(!bucket.contains(&CaseInsensitiveString::from(
            "https://example.com/unvisited"
        )));
        assert!(!bucket.contains(&CaseInsensitiveString::from("https://never-seen.com")));
    }

    /// With neither `disk` nor `bloom` there is no safe automatic remedy, so
    /// entries must **not** be dropped — the set keeps every link and the
    /// one-shot warning latch fires exactly once.
    #[test]
    #[cfg(all(not(feature = "bloom"), not(feature = "disk")))]
    fn test_list_bucket_unbounded_warns_once_and_keeps_entries() {
        let limit = *crate::website::LINKS_VISITED_MEMORY_LIMIT;
        let total = limit.saturating_add(10);

        let mut bucket: ListBucket<CaseInsensitiveString> = ListBucket::new();

        for i in 0..total {
            bucket.insert(CaseInsensitiveString::from(
                format!("https://example.com/{}", i).as_str(),
            ));
        }

        // Nothing is evicted: evicting would create false negatives.
        assert_eq!(bucket.len(), total);
        assert!(bucket.limit_warned, "the warning latch should have fired");

        // Every link is still reported as visited.
        for i in 0..total {
            assert!(bucket.contains(&CaseInsensitiveString::from(
                format!("https://example.com/{}", i).as_str()
            )));
        }

        // `clear` re-arms the latch.
        bucket.clear();
        assert!(!bucket.limit_warned);
    }

    #[test]
    fn test_list_bucket_duplicate_insert() {
        let mut bucket = ListBucket::new();
        bucket.insert(CaseInsensitiveString::from("https://a.com"));
        bucket.insert(CaseInsensitiveString::from("https://a.com"));
        assert_eq!(bucket.len(), 1);
    }
}
