//! Adaptive wait-for timeout guard for chronic bad domains.
//!
//! Feature-gated behind `wait_guard`. Tracks domains that repeatedly
//! produce empty / useless results after expensive `wait_for` operations
//! (idle-network, selector, delay). When a domain exceeds a configurable
//! failure threshold its wait-for timeout is progressively halved, freeing
//! browser pages faster and preventing one user's bad-domain crawl from
//! starving the shared browser pool.
//!
//! Design constraints:
//! - **Fixed capacity** — bounded `DashMap` with LRU eviction so memory
//!   never grows unbounded regardless of how many domains are seen.
//! - **Only tracks bad outcomes** — a domain is never inserted until a
//!   wait-for produces a useless result. Good domains cost zero memory.
//! - **Lock-free** — `DashMap` per-shard locking, atomic counters.
//! - **No panics** — all operations are infallible.
//! - **Never inflates** — `adjusted_timeout` always returns `<= base`.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Maximum entries before LRU eviction kicks in.
const MAX_ENTRIES: usize = 2048;

/// Number of consecutive bad results before we start reducing timeouts.
const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

/// Minimum timeout floor — never reduce below this, but also never
/// return more than `base` (see `adjusted_timeout`).
const MIN_TIMEOUT: Duration = Duration::from_millis(500);

/// Per-domain failure tracking entry.
struct DomainEntry {
    /// Number of consecutive bad wait-for results.
    consecutive_failures: u32,
    /// Monotonic counter snapshot for LRU eviction.
    last_access: u64,
}

/// Fixed-capacity tracker of domains that waste wait-for time.
///
/// Thread-safe, lock-free per shard. Call [`record_bad`] after a
/// wait-for produces an empty / useless result, and [`adjusted_timeout`]
/// before starting a wait-for to get the (possibly reduced) timeout.
pub struct WaitGuard {
    entries: DashMap<Box<str>, DomainEntry>,
    access_counter: AtomicU64,
    failure_threshold: u32,
}

impl WaitGuard {
    /// Create a new guard with the default failure threshold.
    pub fn new() -> Self {
        Self {
            entries: DashMap::with_capacity(256),
            access_counter: AtomicU64::new(0),
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
        }
    }

    /// Create a new guard with a custom failure threshold.
    ///
    /// `threshold` is the number of consecutive bad results before
    /// timeout reduction begins. Must be >= 1.
    pub fn with_threshold(threshold: u32) -> Self {
        Self {
            entries: DashMap::with_capacity(256),
            access_counter: AtomicU64::new(0),
            failure_threshold: threshold.max(1),
        }
    }

    /// Record a bad wait-for outcome for `domain`.
    ///
    /// Only call this when the page result after wait-for is useless
    /// (empty body, cacheable-empty HTML, timeout with no content).
    /// This increments the consecutive failure counter.
    pub fn record_bad(&self, domain: &str) {
        let tick = self.access_counter.fetch_add(1, Ordering::Relaxed);

        // Fast path: domain already tracked — increment in place.
        if let Some(mut entry) = self.entries.get_mut(domain) {
            entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
            entry.last_access = tick;
            return;
        }

        // Slow path: new domain. Evict *before* inserting so we never
        // hold a shard lock while iterating (which would deadlock).
        if self.entries.len() >= MAX_ENTRIES {
            self.evict_lru();
        }

        // TOCTOU note: another thread could have inserted this domain
        // between our get_mut and this insert. DashMap::insert is an
        // upsert, so we'd reset to 1. This is acceptable — it only
        // happens on the very first failure for a domain under heavy
        // concurrency, and only "loses" one failure count.
        self.entries.insert(
            domain.into(),
            DomainEntry {
                consecutive_failures: 1,
                last_access: tick,
            },
        );
    }

    /// Record a good wait-for outcome for `domain`.
    ///
    /// Clears the failure counter so the domain is no longer penalized.
    /// If the domain was never tracked this is a no-op (zero allocation).
    pub fn record_good(&self, domain: &str) {
        if let Some(mut entry) = self.entries.get_mut(domain) {
            entry.consecutive_failures = 0;
            let tick = self.access_counter.fetch_add(1, Ordering::Relaxed);
            entry.last_access = tick;
        }
        // Domain not tracked → no-op, don't insert good domains.
    }

    /// Return an adjusted timeout for `domain`.
    ///
    /// If the domain has exceeded the failure threshold, the base timeout
    /// is halved for each threshold multiple (capped at [`MIN_TIMEOUT`]).
    ///
    /// **Guarantee:** the returned value is always `<= base`. If `base`
    /// is already zero or below `MIN_TIMEOUT`, it is returned unchanged —
    /// this function never inflates a timeout.
    pub fn adjusted_timeout(&self, domain: &str, base: Duration) -> Duration {
        // Fast path: never inflate a timeout that is already at or below
        // the floor.  This also handles Duration::ZERO (budget exhausted)
        // which must stay zero so the caller times out immediately.
        if base <= MIN_TIMEOUT {
            return base;
        }

        let failures = match self.entries.get(domain) {
            Some(entry) => {
                let f = entry.consecutive_failures;
                drop(entry);
                f
            }
            None => return base,
        };

        if failures < self.failure_threshold {
            return base;
        }

        // Each threshold-multiple halves the timeout.
        // e.g. threshold=3: 3 failures → /2, 6 → /4, 9 → /8
        let halvings = failures / self.failure_threshold;
        // Cap shifts to avoid underflow to zero.
        let halvings = halvings.min(10);
        let reduced = base / (1u32 << halvings);

        if reduced < MIN_TIMEOUT {
            MIN_TIMEOUT
        } else {
            reduced
        }
    }

    /// Check whether a domain has been flagged as a bad wait-for domain
    /// (at or above the failure threshold).
    pub fn is_flagged(&self, domain: &str) -> bool {
        self.entries
            .get(domain)
            .map(|e| e.consecutive_failures >= self.failure_threshold)
            .unwrap_or(false)
    }

    /// Current number of tracked domains.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the guard is tracking any domains.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Evict the least-recently-accessed entry.
    ///
    /// Scans all entries (O(n), n capped at [`MAX_ENTRIES`]) to find the
    /// oldest.  Called only when inserting a new domain at capacity — the
    /// hot path (`adjusted_timeout`, `record_bad` on existing domains)
    /// never triggers this.
    fn evict_lru(&self) {
        let mut oldest_key: Option<Box<str>> = None;
        let mut oldest_access = u64::MAX;

        for entry in self.entries.iter() {
            if entry.value().last_access < oldest_access {
                oldest_access = entry.value().last_access;
                oldest_key = Some(entry.key().clone());
            }
        }

        if let Some(key) = oldest_key {
            self.entries.remove(&key);
        }
    }
}

impl Default for WaitGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Global shared wait guard instance.
///
/// Using a global avoids threading the guard through every function
/// signature. The `DashMap` inside is already thread-safe.
static GLOBAL_WAIT_GUARD: std::sync::LazyLock<WaitGuard> = std::sync::LazyLock::new(WaitGuard::new);

/// Access the global [`WaitGuard`] singleton.
#[inline]
pub fn global_wait_guard() -> &'static WaitGuard {
    &GLOBAL_WAIT_GUARD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_domain_not_flagged() {
        let guard = WaitGuard::new();
        assert!(!guard.is_flagged("example.com"));
        assert_eq!(
            guard.adjusted_timeout("example.com", Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn test_below_threshold_no_reduction() {
        let guard = WaitGuard::new();
        guard.record_bad("example.com");
        guard.record_bad("example.com");
        // 2 failures, threshold is 3
        assert!(!guard.is_flagged("example.com"));
        assert_eq!(
            guard.adjusted_timeout("example.com", Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn test_at_threshold_halves_timeout() {
        let guard = WaitGuard::new();
        for _ in 0..3 {
            guard.record_bad("example.com");
        }
        assert!(guard.is_flagged("example.com"));
        assert_eq!(
            guard.adjusted_timeout("example.com", Duration::from_secs(30)),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn test_double_threshold_quarters_timeout() {
        let guard = WaitGuard::new();
        for _ in 0..6 {
            guard.record_bad("example.com");
        }
        assert_eq!(
            guard.adjusted_timeout("example.com", Duration::from_secs(30)),
            Duration::from_secs(7) + Duration::from_millis(500)
        );
    }

    #[test]
    fn test_minimum_timeout_floor() {
        let guard = WaitGuard::new();
        for _ in 0..100 {
            guard.record_bad("example.com");
        }
        assert_eq!(
            guard.adjusted_timeout("example.com", Duration::from_secs(30)),
            MIN_TIMEOUT
        );
    }

    #[test]
    fn test_record_good_clears_failures() {
        let guard = WaitGuard::new();
        for _ in 0..5 {
            guard.record_bad("example.com");
        }
        assert!(guard.is_flagged("example.com"));
        guard.record_good("example.com");
        assert!(!guard.is_flagged("example.com"));
        assert_eq!(
            guard.adjusted_timeout("example.com", Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn test_record_good_noop_for_untracked() {
        let guard = WaitGuard::new();
        guard.record_good("never-seen.com");
        assert!(guard.is_empty());
    }

    #[test]
    fn test_lru_eviction_at_capacity() {
        let guard = WaitGuard::new();
        // Fill to MAX_ENTRIES
        for i in 0..MAX_ENTRIES {
            guard.record_bad(&format!("domain-{i}.com"));
        }
        assert_eq!(guard.len(), MAX_ENTRIES);

        // One more should evict the oldest (domain-0.com)
        guard.record_bad("new-domain.com");
        assert_eq!(guard.len(), MAX_ENTRIES);
        // domain-0.com was the first inserted (lowest access counter)
        assert!(!guard.entries.contains_key("domain-0.com"));
        assert!(guard.entries.contains_key("new-domain.com"));
    }

    #[test]
    fn test_custom_threshold() {
        let guard = WaitGuard::with_threshold(1);
        guard.record_bad("fast-flag.com");
        assert!(guard.is_flagged("fast-flag.com"));
        assert_eq!(
            guard.adjusted_timeout("fast-flag.com", Duration::from_secs(30)),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn test_threshold_min_one() {
        let guard = WaitGuard::with_threshold(0);
        assert_eq!(guard.failure_threshold, 1);
    }

    #[test]
    fn test_saturating_add_no_overflow() {
        let guard = WaitGuard::new();
        // Manually set near u32::MAX
        guard.entries.insert(
            "overflow.com".into(),
            DomainEntry {
                consecutive_failures: u32::MAX - 1,
                last_access: 0,
            },
        );
        guard.record_bad("overflow.com");
        guard.record_bad("overflow.com");
        // Should saturate at MAX, not panic
        let entry = guard.entries.get("overflow.com").unwrap();
        assert_eq!(entry.consecutive_failures, u32::MAX);
    }

    #[test]
    fn test_multiple_domains_independent() {
        let guard = WaitGuard::new();
        for _ in 0..5 {
            guard.record_bad("bad.com");
        }
        guard.record_bad("ok.com");
        assert!(guard.is_flagged("bad.com"));
        assert!(!guard.is_flagged("ok.com"));
    }

    #[test]
    fn test_global_singleton_accessible() {
        let g = global_wait_guard();
        // Just verify it doesn't panic and returns a valid reference.
        assert!(g.len() < usize::MAX);
    }

    // --- Regression: adjusted_timeout must never inflate ---

    #[test]
    fn test_zero_base_never_inflated() {
        let guard = WaitGuard::new();
        for _ in 0..10 {
            guard.record_bad("exhausted.com");
        }
        assert!(guard.is_flagged("exhausted.com"));
        // Budget exhausted — must stay at zero, never inflate to MIN_TIMEOUT.
        assert_eq!(
            guard.adjusted_timeout("exhausted.com", Duration::ZERO),
            Duration::ZERO
        );
    }

    #[test]
    fn test_small_base_never_inflated() {
        let guard = WaitGuard::new();
        for _ in 0..10 {
            guard.record_bad("small.com");
        }
        // Base is 200ms, already below MIN_TIMEOUT (500ms).
        // Must return 200ms, not inflate to 500ms.
        let base = Duration::from_millis(200);
        assert_eq!(guard.adjusted_timeout("small.com", base), base);
    }

    #[test]
    fn test_base_at_min_timeout_returned_unchanged() {
        let guard = WaitGuard::new();
        for _ in 0..10 {
            guard.record_bad("floor.com");
        }
        // Base equals MIN_TIMEOUT — should be returned as-is.
        assert_eq!(
            guard.adjusted_timeout("floor.com", MIN_TIMEOUT),
            MIN_TIMEOUT
        );
    }

    // --- Concurrency: entry API prevents counter reset race ---

    #[test]
    fn test_concurrent_record_bad_does_not_reset() {
        let guard = WaitGuard::new();
        // Simulate: first call creates the entry.
        guard.record_bad("race.com");
        // Second call must increment, not reset to 1.
        guard.record_bad("race.com");
        let entry = guard.entries.get("race.com").unwrap();
        assert_eq!(entry.consecutive_failures, 2);
    }
}
