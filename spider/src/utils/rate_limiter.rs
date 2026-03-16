//! Per-domain token bucket rate limiter.
//!
//! Feature-gated behind `rate_limit`. Provides cooperative rate limiting
//! across concurrent crawl tasks targeting the same domain.

#[cfg(feature = "rate_limit")]
mod inner {
    use crate::compact_str::CompactString;
    use dashmap::DashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    /// Maximum number of tracked domains before LRU eviction kicks in.
    const MAX_ENTRIES: usize = 10_000;

    /// Per-domain token bucket state.
    struct TokenBucket {
        /// Current number of available tokens (fractional).
        tokens: f64,
        /// Last time tokens were refilled.
        last_refill: Instant,
        /// Tokens added per second.
        rate: f64,
        /// Maximum tokens (burst capacity).
        burst: u32,
        /// Monotonic counter for LRU eviction — higher = more recently used.
        last_access: u64,
    }

    impl TokenBucket {
        fn new(rate: f64, burst: u32, access_counter: u64) -> Self {
            Self {
                tokens: burst as f64,
                last_refill: Instant::now(),
                rate,
                burst,
                last_access: access_counter,
            }
        }

        /// Refill tokens based on elapsed time since last refill.
        fn refill(&mut self) {
            let now = Instant::now();
            let elapsed = now.duration_since(self.last_refill).as_secs_f64();
            if elapsed > 0.0 {
                let added = elapsed * self.rate;
                self.tokens = (self.tokens + added).min(self.burst as f64);
                self.last_refill = now;
            }
        }

        /// Try to consume one token. Returns `Duration::ZERO` on success,
        /// or the wait time needed before a token is available.
        fn try_acquire(&mut self) -> Duration {
            self.refill();
            if self.tokens >= 1.0 {
                self.tokens -= 1.0;
                Duration::ZERO
            } else {
                let deficit = 1.0 - self.tokens;
                if self.rate > 0.0 {
                    Duration::from_secs_f64(deficit / self.rate)
                } else {
                    // Zero rate means fully throttled — return a large but bounded wait.
                    Duration::from_secs(120)
                }
            }
        }
    }

    /// Per-domain rate limiter using the token bucket algorithm.
    ///
    /// Thread-safe: uses `DashMap` for concurrent access with minimal lock hold time.
    /// No async in the core — callers sleep the returned `Duration` themselves.
    pub struct DomainRateLimiter {
        buckets: DashMap<CompactString, TokenBucket>,
        default_rate: f64,
        default_burst: u32,
        /// Monotonically increasing counter for LRU tracking.
        access_counter: AtomicU64,
    }

    impl DomainRateLimiter {
        /// Create a new rate limiter.
        ///
        /// - `default_rate`: tokens per second for new domains (clamped to `[0.01, 1_000_000]`).
        /// - `default_burst`: max burst tokens for new domains (clamped to `[1, 10_000]`).
        pub fn new(default_rate: f64, default_burst: u32) -> Self {
            Self {
                buckets: DashMap::new(),
                default_rate: default_rate.clamp(0.01, 1_000_000.0),
                default_burst: default_burst.clamp(1, 10_000),
                access_counter: AtomicU64::new(0),
            }
        }

        /// Acquire a token for `domain`. Returns how long the caller should wait
        /// before making the request. `Duration::ZERO` means proceed immediately.
        pub fn acquire(&self, domain: &str) -> Duration {
            let counter = self.access_counter.fetch_add(1, Ordering::Relaxed);
            let key = CompactString::new(domain);

            // Fast path: domain already tracked.
            if let Some(mut bucket) = self.buckets.get_mut(&key) {
                bucket.last_access = counter;
                return bucket.try_acquire();
            }

            // Slow path: insert new bucket, evicting if over capacity.
            self.maybe_evict();

            let mut bucket = TokenBucket::new(self.default_rate, self.default_burst, counter);
            let wait = bucket.try_acquire();
            self.buckets.insert(key, bucket);
            wait
        }

        /// Called on HTTP 429: reduce the domain's rate to respect the server's
        /// `Retry-After` duration. The bucket is drained and the rate is adjusted
        /// so roughly one token appears after `retry_after` elapses.
        pub fn throttle(&self, domain: &str, retry_after: Duration) {
            let key = CompactString::new(domain);
            let secs = retry_after.as_secs_f64().max(1.0);
            // New rate: 1 token per retry_after period (at minimum 0.01 t/s).
            let new_rate = (1.0 / secs).max(0.01);

            if let Some(mut bucket) = self.buckets.get_mut(&key) {
                bucket.rate = new_rate;
                bucket.tokens = 0.0;
                bucket.last_refill = Instant::now();
            } else {
                self.maybe_evict();
                let mut bucket = TokenBucket::new(new_rate, self.default_burst, 0);
                bucket.tokens = 0.0;
                self.buckets.insert(key, bucket);
            }
        }

        /// Override the rate for a specific domain (e.g., from robots.txt `Crawl-delay`).
        ///
        /// `rate` is tokens per second, clamped to `[0.01, 1_000_000]`.
        pub fn set_rate(&self, domain: &str, rate: f64) {
            let key = CompactString::new(domain);
            let rate = rate.clamp(0.01, 1_000_000.0);

            if let Some(mut bucket) = self.buckets.get_mut(&key) {
                bucket.rate = rate;
            } else {
                self.maybe_evict();
                self.buckets
                    .insert(key, TokenBucket::new(rate, self.default_burst, 0));
            }
        }

        /// Number of tracked domains.
        pub fn len(&self) -> usize {
            self.buckets.len()
        }

        /// Whether the limiter is tracking any domains.
        pub fn is_empty(&self) -> bool {
            self.buckets.is_empty()
        }

        /// Evict the least-recently-used entry if we exceed `MAX_ENTRIES`.
        fn maybe_evict(&self) {
            if self.buckets.len() < MAX_ENTRIES {
                return;
            }

            // Find the entry with the smallest last_access counter.
            let mut oldest_key: Option<CompactString> = None;
            let mut oldest_access = u64::MAX;

            for entry in self.buckets.iter() {
                if entry.value().last_access < oldest_access {
                    oldest_access = entry.value().last_access;
                    oldest_key = Some(entry.key().clone());
                }
            }

            if let Some(key) = oldest_key {
                self.buckets.remove(&key);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::Duration;

        #[test]
        fn test_acquire_immediate_when_tokens_available() {
            let limiter = DomainRateLimiter::new(10.0, 10);
            let wait = limiter.acquire("example.com");
            assert_eq!(wait, Duration::ZERO);
        }

        #[test]
        fn test_acquire_returns_wait_when_exhausted() {
            // 1 token/sec, burst 1 — first acquire succeeds, second must wait.
            let limiter = DomainRateLimiter::new(1.0, 1);
            let first = limiter.acquire("slow.com");
            assert_eq!(first, Duration::ZERO);

            let second = limiter.acquire("slow.com");
            assert!(second > Duration::ZERO, "should need to wait");
            // Should be roughly 1 second (give or take timing).
            assert!(second <= Duration::from_secs(2));
        }

        #[test]
        fn test_different_domains_are_independent() {
            let limiter = DomainRateLimiter::new(1.0, 1);
            let a = limiter.acquire("a.com");
            let b = limiter.acquire("b.com");
            assert_eq!(a, Duration::ZERO);
            assert_eq!(b, Duration::ZERO);
        }

        #[test]
        fn test_throttle_drains_tokens() {
            let limiter = DomainRateLimiter::new(100.0, 100);
            // Pre-populate the bucket.
            let _ = limiter.acquire("throttled.com");

            // Simulate 429 with 10-second retry.
            limiter.throttle("throttled.com", Duration::from_secs(10));
            let wait = limiter.acquire("throttled.com");
            // Tokens were drained, rate is now ~0.1/s, so wait should be significant.
            assert!(wait > Duration::from_millis(500));
        }

        #[test]
        fn test_set_rate_updates_existing() {
            let limiter = DomainRateLimiter::new(10.0, 10);
            let _ = limiter.acquire("custom.com");

            // Set a very slow rate.
            limiter.set_rate("custom.com", 0.1);

            // Exhaust the remaining tokens.
            // After set_rate the bucket still has leftover tokens from init,
            // so drain them first.
            for _ in 0..20 {
                let w = limiter.acquire("custom.com");
                if w > Duration::ZERO {
                    // Confirmed: rate change is in effect.
                    assert!(w > Duration::from_millis(100));
                    return;
                }
            }
            panic!("expected a non-zero wait after lowering rate");
        }

        #[test]
        fn test_set_rate_creates_bucket_if_missing() {
            let limiter = DomainRateLimiter::new(10.0, 10);
            limiter.set_rate("new.com", 5.0);
            assert_eq!(limiter.len(), 1);
        }

        #[test]
        fn test_throttle_creates_bucket_if_missing() {
            let limiter = DomainRateLimiter::new(10.0, 10);
            limiter.throttle("new.com", Duration::from_secs(5));
            assert_eq!(limiter.len(), 1);
            let wait = limiter.acquire("new.com");
            assert!(wait > Duration::ZERO);
        }

        #[test]
        fn test_eviction_at_capacity() {
            // Use a small custom max for testing — we test the eviction logic
            // by filling up and checking that len stays bounded.
            let limiter = DomainRateLimiter::new(10.0, 10);
            // Insert MAX_ENTRIES + 1 domains.
            for i in 0..=MAX_ENTRIES {
                let _ = limiter.acquire(&format!("domain-{i}.com"));
            }
            // Should have evicted at least one.
            assert!(limiter.len() <= MAX_ENTRIES);
        }

        #[test]
        fn test_rate_clamping() {
            // Extreme values should be clamped, not panic.
            let limiter = DomainRateLimiter::new(-100.0, 0);
            let wait = limiter.acquire("clamped.com");
            // With clamped minimum rate (0.01) and burst (1), first acquire should still work.
            assert_eq!(wait, Duration::ZERO);
        }
    }
}

#[cfg(feature = "rate_limit")]
pub use inner::DomainRateLimiter;
