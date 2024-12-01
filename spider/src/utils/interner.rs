use crate::CaseInsensitiveString;
use hashbrown::HashSet;
use std::hash::Hash;

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

#[cfg(all(
    feature = "string_interner_bucket_backend",
    not(feature = "string_interner_string_backend"),
    not(feature = "string_interner_buffer_backend"),
))]
type Backend = string_interner::backend::BucketBackend<SymbolUsize>;

#[cfg(all(
    feature = "string_interner_string_backend",
    not(feature = "string_interner_bucket_backend"),
    not(feature = "string_interner_buffer_backend"),
))]
type Backend = string_interner::backend::StringBackend<SymbolUsize>;

#[cfg(all(
    feature = "string_interner_buffer_backend",
    not(feature = "string_interner_bucket_backend"),
    not(feature = "string_interner_string_backend"),
))]
type Backend = string_interner::backend::BufferBackend<SymbolUsize>;

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
))]
/// The links visited bucket store.
pub struct ListBucket<K = CaseInsensitiveString>
where
    K: Eq + Hash + Clone + AsRef<str>,
{
    pub(crate) links_visited: HashSet<SymbolUsize>,
    pub(crate) interner: StringInterner<Backend>,
    _marker: PhantomData<K>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(not(any(
    feature = "string_interner_bucket_backend",
    feature = "string_interner_string_backend",
    feature = "string_interner_buffer_backend",
)))]
/// The links visited bucket store.
pub struct ListBucket<K = CaseInsensitiveString>
where
    K: Eq + Hash + Clone + AsRef<str>,
{
    pub(crate) links_visited: HashSet<K>,
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

    /// Add a new link to the bucket.
    #[inline(always)]
    pub fn insert(&mut self, link: K) {
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
    }

    /// Does the bucket contain the link.
    #[inline(always)]
    pub fn contains(&self, link: &K) -> bool {
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
    pub fn len(&self) -> usize {
        self.links_visited.len()
    }

    /// The bucket is empty.
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
        self.links_visited.clear()
    }

    /// Get a vector of all the inner values of the links in the bucket.
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
                let symbol = self.interner.get_or_intern(link.as_ref());
                if !self.links_visited.contains(&symbol) {
                    links.insert(link);
                }
            }
        }

        #[cfg(not(any(
            feature = "string_interner_bucket_backend",
            feature = "string_interner_string_backend",
            feature = "string_interner_buffer_backend",
        )))]
        {
            links.extend(msg.difference(&self.links_visited).cloned());
        }
    }

    /// Extend with new links.
    #[inline(always)]
    pub fn extend_with_new_links(&mut self, links: &mut HashSet<K>, s: K)
    where
        K: Clone,
    {
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
