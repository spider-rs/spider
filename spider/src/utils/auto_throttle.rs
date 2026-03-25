//! Latency-based auto-throttle for polite, adaptive crawling.
//!
//! Feature-gated behind `auto_throttle`. Tracks per-domain response latency
//! via exponential moving average (EMA) and dynamically computes a crawl delay
//! so that the target number of concurrent requests remains within a latency
//! window.
//!
//! Inspired by Scrapy's AUTOTHROTTLE — increases delay when servers respond
//! slowly, decreases when they are fast. All operations are lock-free
//! (DashMap + atomics).

#[cfg(feature = "auto_throttle")]
mod inner {
    use crate::compact_str::CompactString;
    use dashmap::DashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// Maximum tracked domains before LRU eviction.
    const MAX_ENTRIES: usize = 10_000;

    /// Default smoothing factor for EMA (0..1). Higher = more responsive.
    const DEFAULT_ALPHA: f64 = 0.15;

    /// Per-domain latency state stored as atomic u64 bit-patterns.
    struct DomainLatency {
        /// EMA of response time in microseconds, stored as f64 bits.
        ema_us: AtomicU64,
        /// Number of samples recorded (saturates at u64::MAX).
        samples: AtomicU64,
        /// Monotonic access counter for LRU eviction.
        last_access: AtomicU64,
    }

    impl DomainLatency {
        fn new(access_counter: u64) -> Self {
            Self {
                ema_us: AtomicU64::new(0),
                samples: AtomicU64::new(0),
                last_access: AtomicU64::new(access_counter),
            }
        }

        /// Load the current EMA in microseconds.
        fn ema_micros(&self) -> f64 {
            f64::from_bits(self.ema_us.load(Ordering::Relaxed))
        }

        /// Record a new latency sample and update the EMA.
        fn record(&self, latency_us: f64, alpha: f64) {
            let prev_count = self.samples.fetch_add(1, Ordering::Relaxed);

            if prev_count == 0 {
                // First sample — seed the EMA.
                self.ema_us.store(latency_us.to_bits(), Ordering::Relaxed);
            } else {
                // CAS loop to atomically update f64 EMA.
                let _ = self
                    .ema_us
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bits| {
                        let old = f64::from_bits(bits);
                        let new = old + alpha * (latency_us - old);
                        // Guard: never store NaN/Inf.
                        if new.is_finite() && new >= 0.0 {
                            Some(new.to_bits())
                        } else {
                            Some(old.to_bits())
                        }
                    });
            }
        }
    }

    /// Configuration for the auto-throttle.
    #[derive(Debug, Clone, PartialEq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct AutoThrottleConfig {
        /// Target concurrency per domain. The throttle aims to keep
        /// `delay ≈ latency / target_concurrency` so that approximately this
        /// many requests are in-flight per domain at steady state.
        /// Default: 2.0.
        pub target_concurrency: f64,
        /// Minimum delay between requests in milliseconds. Default: 0.
        pub min_delay_ms: u64,
        /// Maximum delay between requests in milliseconds. Default: 60_000.
        pub max_delay_ms: u64,
        /// EMA smoothing factor (0..1). Higher = more responsive to latency
        /// changes. Default: 0.15.
        pub alpha: f64,
        /// Whether auto-throttle is enabled. Default: true.
        pub enabled: bool,
    }

    impl Default for AutoThrottleConfig {
        fn default() -> Self {
            Self {
                target_concurrency: 2.0,
                min_delay_ms: 0,
                max_delay_ms: 60_000,
                alpha: DEFAULT_ALPHA,
                enabled: true,
            }
        }
    }

    /// Per-domain auto-throttle that dynamically computes crawl delay.
    ///
    /// Thread-safe: `DashMap` for concurrent access, atomics for per-domain state.
    pub struct AutoThrottle {
        domains: DashMap<CompactString, DomainLatency>,
        config: AutoThrottleConfig,
        /// Monotonically increasing counter for LRU.
        access_counter: AtomicU64,
    }

    impl AutoThrottle {
        /// Create a new auto-throttle with the given configuration.
        pub fn new(config: AutoThrottleConfig) -> Self {
            Self {
                domains: DashMap::with_capacity(64),
                config,
                access_counter: AtomicU64::new(0),
            }
        }

        /// Create with default configuration.
        pub fn with_defaults() -> Self {
            Self::new(AutoThrottleConfig::default())
        }

        /// Record a response latency for a domain.
        ///
        /// Call this after each successful (or failed) fetch with the elapsed
        /// wall-clock time. The EMA is updated atomically.
        pub fn record_latency(&self, domain: &str, latency: Duration) {
            let us = latency.as_micros() as f64;
            let counter = self.access_counter.fetch_add(1, Ordering::Relaxed);
            let key = CompactString::new(domain);
            let alpha = self.config.alpha.clamp(0.01, 1.0);

            if let Some(entry) = self.domains.get(&key) {
                entry.last_access.store(counter, Ordering::Relaxed);
                entry.record(us, alpha);
            } else {
                self.maybe_evict();
                let entry = DomainLatency::new(counter);
                entry.record(us, alpha);
                self.domains.insert(key, entry);
            }
        }

        /// Compute the adaptive delay for a domain.
        ///
        /// Formula: `delay = ema_latency / target_concurrency`, clamped to
        /// `[min_delay_ms, max_delay_ms]`.
        ///
        /// Returns `Duration::ZERO` if no samples have been recorded yet
        /// (cold-start: don't delay until we have data).
        pub fn delay_for(&self, domain: &str) -> Duration {
            if !self.config.enabled {
                return Duration::ZERO;
            }

            let key = CompactString::new(domain);

            let ema_us = match self.domains.get(&key) {
                Some(entry) => {
                    if entry.samples.load(Ordering::Relaxed) == 0 {
                        return Duration::ZERO;
                    }
                    entry.ema_micros()
                }
                None => return Duration::ZERO,
            };

            let target = self.config.target_concurrency.max(0.1);
            let delay_us = ema_us / target;
            let delay_ms = (delay_us / 1000.0) as u64;
            let clamped = delay_ms.clamp(self.config.min_delay_ms, self.config.max_delay_ms);

            Duration::from_millis(clamped)
        }

        /// Get the current EMA latency for a domain in milliseconds.
        /// Returns `None` if no samples recorded.
        pub fn latency_ms(&self, domain: &str) -> Option<f64> {
            let key = CompactString::new(domain);
            self.domains.get(&key).and_then(|entry| {
                if entry.samples.load(Ordering::Relaxed) == 0 {
                    None
                } else {
                    Some(entry.ema_micros() / 1000.0)
                }
            })
        }

        /// Number of tracked domains.
        pub fn len(&self) -> usize {
            self.domains.len()
        }

        /// Whether the throttle is tracking any domains.
        pub fn is_empty(&self) -> bool {
            self.domains.is_empty()
        }

        /// Evict the least-recently-used entry if over capacity.
        fn maybe_evict(&self) {
            if self.domains.len() < MAX_ENTRIES {
                return;
            }

            let mut oldest_key: Option<CompactString> = None;
            let mut oldest_access = u64::MAX;

            for entry in self.domains.iter() {
                let access = entry.value().last_access.load(Ordering::Relaxed);
                if access < oldest_access {
                    oldest_access = access;
                    oldest_key = Some(entry.key().clone());
                }
            }

            if let Some(key) = oldest_key {
                self.domains.remove(&key);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::Duration;

        #[test]
        fn cold_start_returns_zero_delay() {
            let at = AutoThrottle::with_defaults();
            assert_eq!(at.delay_for("example.com"), Duration::ZERO);
        }

        #[test]
        fn first_sample_seeds_ema() {
            let at = AutoThrottle::with_defaults();
            at.record_latency("example.com", Duration::from_millis(200));

            let lat = at.latency_ms("example.com").unwrap();
            assert!((lat - 200.0).abs() < 1.0, "expected ~200ms, got {lat}");
        }

        #[test]
        fn ema_converges_toward_new_value() {
            let config = AutoThrottleConfig {
                alpha: 0.5,
                ..Default::default()
            };
            let at = AutoThrottle::new(config);

            // Seed with 100ms
            at.record_latency("a.com", Duration::from_millis(100));
            // Record 300ms → EMA should move toward 300
            at.record_latency("a.com", Duration::from_millis(300));

            let lat = at.latency_ms("a.com").unwrap();
            // EMA = 100 + 0.5*(300-100) = 200
            assert!((lat - 200.0).abs() < 5.0, "expected ~200ms, got {lat}");
        }

        #[test]
        fn delay_respects_target_concurrency() {
            let config = AutoThrottleConfig {
                target_concurrency: 4.0,
                min_delay_ms: 0,
                max_delay_ms: 60_000,
                ..Default::default()
            };
            let at = AutoThrottle::new(config);

            // 400ms latency, target concurrency 4 → delay = 400/4 = 100ms
            at.record_latency("fast.com", Duration::from_millis(400));
            let delay = at.delay_for("fast.com");
            assert!(
                delay.as_millis() >= 90 && delay.as_millis() <= 110,
                "expected ~100ms delay, got {:?}",
                delay
            );
        }

        #[test]
        fn delay_clamped_to_min_max() {
            let config = AutoThrottleConfig {
                target_concurrency: 1.0,
                min_delay_ms: 50,
                max_delay_ms: 500,
                ..Default::default()
            };
            let at = AutoThrottle::new(config);

            // Very fast server: 5ms → delay = 5/1 = 5ms, but min is 50
            at.record_latency("fast.com", Duration::from_millis(5));
            assert_eq!(at.delay_for("fast.com").as_millis(), 50);

            // Very slow server: 2000ms → delay = 2000/1 = 2000ms, but max is 500
            at.record_latency("slow.com", Duration::from_millis(2000));
            assert_eq!(at.delay_for("slow.com").as_millis(), 500);
        }

        #[test]
        fn disabled_returns_zero() {
            let config = AutoThrottleConfig {
                enabled: false,
                ..Default::default()
            };
            let at = AutoThrottle::new(config);
            at.record_latency("example.com", Duration::from_millis(500));
            assert_eq!(at.delay_for("example.com"), Duration::ZERO);
        }

        #[test]
        fn different_domains_independent() {
            let at = AutoThrottle::with_defaults();
            at.record_latency("a.com", Duration::from_millis(100));
            at.record_latency("b.com", Duration::from_millis(1000));

            let a = at.latency_ms("a.com").unwrap();
            let b = at.latency_ms("b.com").unwrap();
            assert!(a < 200.0);
            assert!(b > 800.0);
        }

        #[test]
        fn eviction_at_capacity() {
            let at = AutoThrottle::with_defaults();
            for i in 0..=MAX_ENTRIES {
                at.record_latency(&format!("domain-{i}.com"), Duration::from_millis(50));
            }
            assert!(at.len() <= MAX_ENTRIES);
        }

        #[test]
        fn no_panic_on_zero_latency() {
            let at = AutoThrottle::with_defaults();
            at.record_latency("zero.com", Duration::ZERO);
            let delay = at.delay_for("zero.com");
            assert_eq!(delay, Duration::ZERO);
        }

        #[test]
        fn no_panic_on_extreme_latency() {
            let at = AutoThrottle::with_defaults();
            at.record_latency("extreme.com", Duration::from_secs(3600));
            // Should be clamped to max_delay_ms (60s)
            let delay = at.delay_for("extreme.com");
            assert!(delay <= Duration::from_millis(60_000));
        }

        #[test]
        fn concurrent_recording_no_panic() {
            use std::sync::Arc;
            let at = Arc::new(AutoThrottle::with_defaults());
            let handles: Vec<_> = (0..8)
                .map(|t| {
                    let at = at.clone();
                    std::thread::spawn(move || {
                        for i in 0..100 {
                            at.record_latency(
                                "shared.com",
                                Duration::from_millis(50 + (t * 10) + i),
                            );
                            let _ = at.delay_for("shared.com");
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            assert!(at.latency_ms("shared.com").is_some());
        }
    }
}

#[cfg(feature = "auto_throttle")]
pub use inner::{AutoThrottle, AutoThrottleConfig};
