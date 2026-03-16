//! Prioritized URL frontier with deduplication.
//!
//! Uses a max-heap (`BinaryHeap`) so the highest-priority URL is always
//! popped first. An optional domain round-robin mode prefers switching
//! domains on consecutive pops.

use case_insensitive_string::compact_str::CompactString;
use case_insensitive_string::CaseInsensitiveString;
use hashbrown::HashSet;
use std::cmp::Ordering as CmpOrdering;
use std::collections::BinaryHeap;

/// A URL annotated with a priority score for heap ordering.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ScoredUrl {
    /// Higher values are popped first.
    pub priority: i32,
    /// The URL itself.
    pub url: CaseInsensitiveString,
}

impl Ord for ScoredUrl {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        self.priority
            .cmp(&other.priority)
            .then_with(|| self.url.cmp(&other.url))
    }
}

impl PartialOrd for ScoredUrl {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

/// Prioritized URL frontier with dedup and optional domain round-robin.
pub struct UrlFrontier {
    heap: BinaryHeap<ScoredUrl>,
    visited: HashSet<CompactString>,
    round_robin: bool,
    last_domain: Option<CompactString>,
    /// Temporary buffer used by round-robin logic to hold skipped entries.
    rr_buf: Vec<ScoredUrl>,
}

impl UrlFrontier {
    /// Create a new empty frontier.
    ///
    /// If `round_robin` is `true`, consecutive `pop()` calls prefer returning
    /// URLs from different domains.
    pub fn new(round_robin: bool) -> Self {
        Self {
            heap: BinaryHeap::new(),
            visited: HashSet::new(),
            round_robin,
            last_domain: None,
            rr_buf: Vec::new(),
        }
    }

    /// Push a URL with the given priority. Returns `true` if the URL was
    /// inserted (i.e. it was not already visited/enqueued).
    pub fn push(&mut self, url: CaseInsensitiveString, priority: i32) -> bool {
        let key = CompactString::new(url.inner());
        if !self.visited.insert(key) {
            return false;
        }
        self.heap.push(ScoredUrl { priority, url });
        true
    }

    /// Pop the highest-priority URL.
    ///
    /// When `round_robin` is enabled, this will skip URLs from the same domain
    /// as the previous pop (re-enqueueing them) until a different domain is
    /// found or no alternative exists.
    pub fn pop(&mut self) -> Option<CaseInsensitiveString> {
        if !self.round_robin {
            return self.heap.pop().map(|s| s.url);
        }

        // Round-robin: try to find a URL from a different domain.
        let last = self.last_domain.as_deref();

        let mut found: Option<ScoredUrl> = None;

        while let Some(entry) = self.heap.pop() {
            let domain = extract_domain(entry.url.inner());
            let same = match last {
                Some(prev) => domain.as_str() == prev,
                None => false,
            };

            if same && found.is_none() {
                // Same domain — stash and keep looking.
                self.rr_buf.push(entry);
            } else {
                // Different domain (or we already stashed entries and this is also
                // same-domain but we've exhausted alternatives — handled below).
                found = Some(entry);
                break;
            }
        }

        // If we couldn't find a different domain, fall back to the first stashed.
        if found.is_none() {
            found = if self.rr_buf.is_empty() {
                None
            } else {
                Some(self.rr_buf.remove(0))
            };
        }

        // Put stashed items back.
        for item in self.rr_buf.drain(..) {
            self.heap.push(item);
        }

        if let Some(ref entry) = found {
            self.last_domain = Some(extract_domain(entry.url.inner()));
        }

        found.map(|s| s.url)
    }

    /// Bulk insert URLs with the same priority.
    pub fn extend_with_priority(
        &mut self,
        urls: impl Iterator<Item = CaseInsensitiveString>,
        priority: i32,
    ) {
        for url in urls {
            self.push(url, priority);
        }
    }

    /// Number of URLs currently in the frontier (not yet popped).
    #[inline]
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Whether the frontier is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

/// High-value path segments that receive a priority bonus.
const HIGH_VALUE: &[&str] = &["product", "article", "item", "page"];

/// Low-value path segments that receive a priority penalty.
const LOW_VALUE: &[&str] = &["legal", "privacy", "terms", "cookie", "disclaimer"];

/// Score a URL for frontier priority.
///
/// * Base = `1000 - depth * 100`
/// * +50 for each high-value path segment match
/// * -200 for each low-value path segment match
/// * Clamped to `[0, 2000]`
pub fn score_url(url: &str, depth: u32) -> i32 {
    let base: i32 = 1000i32.saturating_sub((depth as i32).saturating_mul(100));

    // Extract the path portion (after the authority, before query/fragment).
    let path = url_path(url);
    let path_lower = path.to_ascii_lowercase();

    let mut score = base;

    for seg in HIGH_VALUE {
        if path_lower.contains(seg) {
            score = score.saturating_add(50);
        }
    }

    for seg in LOW_VALUE {
        if path_lower.contains(seg) {
            score = score.saturating_sub(200);
        }
    }

    score.clamp(0, 2000)
}

/// Extract the domain (host) from a URL string. Returns empty string on parse
/// failure.
fn extract_domain(url: &str) -> CompactString {
    // Fast path: find "://" then next '/' or end.
    if let Some(start) = url.find("://") {
        let after = start + 3;
        let rest = &url[after..];
        let end = rest.find('/').unwrap_or(rest.len());
        // Strip port if present.
        let host = &rest[..end];
        let host = host.split(':').next().unwrap_or(host);
        CompactString::new(host)
    } else {
        CompactString::default()
    }
}

/// Extract the path portion of a URL (between host and query/fragment).
fn url_path(url: &str) -> &str {
    if let Some(start) = url.find("://") {
        let after = start + 3;
        let rest = &url[after..];
        if let Some(slash) = rest.find('/') {
            let path_start = after + slash;
            let remaining = &url[path_start..];
            let end = remaining
                .find('?')
                .unwrap_or_else(|| remaining.find('#').unwrap_or(remaining.len()));
            &remaining[..end]
        } else {
            "/"
        }
    } else {
        url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cis(s: &str) -> CaseInsensitiveString {
        CaseInsensitiveString::from(s)
    }

    #[test]
    fn push_dedup() {
        let mut f = UrlFrontier::new(false);
        assert!(f.push(cis("https://example.com/a"), 100));
        assert!(!f.push(cis("https://example.com/a"), 200)); // duplicate
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn pop_highest_priority_first() {
        let mut f = UrlFrontier::new(false);
        f.push(cis("https://example.com/low"), 10);
        f.push(cis("https://example.com/high"), 500);
        f.push(cis("https://example.com/mid"), 100);

        let first = f.pop().unwrap();
        assert_eq!(first, cis("https://example.com/high"));
        let second = f.pop().unwrap();
        assert_eq!(second, cis("https://example.com/mid"));
    }

    #[test]
    fn extend_with_priority_bulk() {
        let mut f = UrlFrontier::new(false);
        let urls = vec![
            cis("https://a.com/1"),
            cis("https://b.com/2"),
            cis("https://a.com/1"), // dup
        ];
        f.extend_with_priority(urls.into_iter(), 50);
        assert_eq!(f.len(), 2);
    }

    #[test]
    fn score_url_depth_and_segments() {
        // Depth 0, high-value segment
        let s = score_url("https://shop.com/product/widget", 0);
        assert_eq!(s, 1050);

        // Depth 0, low-value segment
        let s = score_url("https://shop.com/legal/privacy", 0);
        // 1000 - 200(legal) - 200(privacy) = 600
        assert_eq!(s, 600);

        // Deep page
        let s = score_url("https://shop.com/deep", 15);
        // 1000 - 1500 = -500 → clamped to 0
        assert_eq!(s, 0);
    }

    #[test]
    fn round_robin_alternates_domains() {
        let mut f = UrlFrontier::new(true);
        f.push(cis("https://a.com/1"), 100);
        f.push(cis("https://a.com/2"), 90);
        f.push(cis("https://b.com/1"), 95);

        // First pop: a.com/1 (highest prio)
        let first = f.pop().unwrap();
        assert_eq!(first, cis("https://a.com/1"));

        // Second pop: should prefer b.com (different domain) even though a.com/2
        // has lower prio than b.com/1.
        let second = f.pop().unwrap();
        assert_eq!(second, cis("https://b.com/1"));

        // Third: back to a.com
        let third = f.pop().unwrap();
        assert_eq!(third, cis("https://a.com/2"));
    }

    #[test]
    fn pop_empty_returns_none() {
        let mut f = UrlFrontier::new(false);
        assert!(f.pop().is_none());
        assert!(f.is_empty());
    }

    #[test]
    fn extract_domain_various() {
        assert_eq!(
            extract_domain("https://www.example.com/path"),
            CompactString::new("www.example.com")
        );
        assert_eq!(
            extract_domain("http://localhost:8080/test"),
            CompactString::new("localhost")
        );
        assert_eq!(extract_domain("no-scheme"), CompactString::default());
    }

    #[test]
    fn score_url_clamped() {
        // Max clamp
        let s = score_url("https://x.com/product/article/item/page", 0);
        // 1000 + 50*4 = 1200, within 2000
        assert_eq!(s, 1200);

        // Min clamp at depth 20 with low-value
        let s = score_url("https://x.com/legal", 20);
        assert_eq!(s, 0);
    }
}
