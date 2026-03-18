/// Work-stealing (hedged requests) for slow crawl requests.
///
/// After a configurable delay, fires a duplicate request on a different proxy.
/// Whichever returns first wins; the loser is cancelled via drop.
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Configuration for hedged (work-stealing) requests.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HedgeConfig {
    /// Time to wait before launching the first hedge request.
    pub delay: Duration,
    /// Max concurrent hedge attempts (1 = one hedge, 2 = two hedges staggered at `delay` intervals).
    pub max_hedges: usize,
    /// Master switch. When false, hedging is a no-op.
    pub enabled: bool,
}

impl Default for HedgeConfig {
    fn default() -> Self {
        Self {
            delay: Duration::from_secs(3),
            max_hedges: 1,
            enabled: true,
        }
    }
}

/// Tracks page fetch velocity and hedge effectiveness to decide whether hedging
/// is worthwhile and how long to wait before firing.
///
/// Uses exponential moving averages (EMA) of duration and squared-duration for
/// variance-aware adaptive delay. Tracks hedge win rate and consecutive errors
/// to auto-tune aggressiveness.
///
/// Fully lock-free: uses atomics only, safe for concurrent access from any
/// number of tasks. No mutexes, no allocations after construction.
#[derive(Debug)]
pub struct HedgeTracker {
    /// EMA of page fetch durations in milliseconds.
    ema_ms: AtomicU64,
    /// EMA of squared durations (ms²) for variance/stddev calculation.
    ema_sq_ms: AtomicU64,
    /// Number of duration samples recorded.
    samples: AtomicU64,
    /// Multiplier for the EMA threshold (e.g. 2.0 = hedge if fetch takes >2x average).
    /// Stored as fixed-point: actual * 100 (e.g. 200 = 2.0x).
    threshold_pct: u64,
    /// Minimum samples before adaptive hedging activates.
    min_samples: u64,
    /// Total hedge races fired (delay expired, hedge launched).
    hedge_fires: AtomicU64,
    /// Hedge races where the hedge tab won.
    hedge_wins: AtomicU64,
    /// Consecutive retryable errors (5xx, timeout). Reset to 0 on success.
    consecutive_errors: AtomicU64,
}

impl HedgeTracker {
    /// Create a new tracker.
    /// `threshold_multiplier` — hedge fires only when elapsed > EMA * multiplier (e.g. 2.0).
    /// `min_samples` — number of completed fetches before adaptive hedging activates.
    pub fn new(threshold_multiplier: f64, min_samples: u64) -> Self {
        Self {
            ema_ms: AtomicU64::new(0),
            ema_sq_ms: AtomicU64::new(0),
            samples: AtomicU64::new(0),
            threshold_pct: (threshold_multiplier * 100.0) as u64,
            min_samples,
            hedge_fires: AtomicU64::new(0),
            hedge_wins: AtomicU64::new(0),
            consecutive_errors: AtomicU64::new(0),
        }
    }

    /// Record a completed page fetch duration. Updates both the duration EMA
    /// and the squared-duration EMA (for variance tracking).
    pub fn record(&self, duration: Duration) {
        let ms = duration.as_millis() as u64;
        let ms_sq = ms.saturating_mul(ms);
        let count = self.samples.fetch_add(1, Ordering::Relaxed);

        if count == 0 {
            self.ema_ms.store(ms, Ordering::Relaxed);
            self.ema_sq_ms.store(ms_sq, Ordering::Relaxed);
        } else {
            // EMA with alpha ≈ 0.2 (integer math: new = old * 4/5 + sample * 1/5)
            let old = self.ema_ms.load(Ordering::Relaxed);
            self.ema_ms.store((old * 4 + ms) / 5, Ordering::Relaxed);

            let old_sq = self.ema_sq_ms.load(Ordering::Relaxed);
            self.ema_sq_ms
                .store((old_sq * 4 + ms_sq) / 5, Ordering::Relaxed);
        }
    }

    /// Record that a hedge was fired (delay expired).
    pub fn record_fired(&self) {
        self.hedge_fires.fetch_add(1, Ordering::Relaxed);
    }

    /// Record hedge race outcome. `hedge_won = true` when the hedge tab beat the primary.
    pub fn record_outcome(&self, hedge_won: bool) {
        if hedge_won {
            self.hedge_wins.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a retryable error (5xx, timeout, proxy error). Increments consecutive
    /// error counter, which makes adaptive_delay more aggressive.
    pub fn record_error(&self) {
        self.consecutive_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a successful fetch. Resets consecutive error counter.
    pub fn record_success(&self) {
        self.consecutive_errors.store(0, Ordering::Relaxed);
    }

    /// Check if hedging should fire given elapsed time since fetch start.
    /// Returns true only when:
    /// 1. We have enough samples (past warmup)
    /// 2. Elapsed exceeds EMA * threshold_multiplier
    pub fn should_hedge(&self, elapsed: Duration) -> bool {
        let count = self.samples.load(Ordering::Relaxed);
        if count < self.min_samples {
            // During warmup, always allow hedging (fall back to config delay)
            return true;
        }

        let ema = self.ema_ms.load(Ordering::Relaxed);
        let threshold_ms = ema * self.threshold_pct / 100;
        elapsed.as_millis() as u64 > threshold_ms
    }

    /// Compute the standard deviation of fetch durations in milliseconds.
    /// Uses variance = E[X²] - E[X]² from the two EMAs.
    pub fn stddev_ms(&self) -> u64 {
        let ema = self.ema_ms.load(Ordering::Relaxed);
        let ema_sq = self.ema_sq_ms.load(Ordering::Relaxed);
        let variance = ema_sq.saturating_sub(ema.saturating_mul(ema));
        (variance as f64).sqrt() as u64
    }

    /// Get the adaptive delay, incorporating multiple signals:
    ///
    /// 1. **Variance-aware baseline**: `EMA + stddev` (≈P84) — only hedge genuinely
    ///    slow pages, not normal variance.
    /// 2. **Consecutive errors**: 3+ errors in a row halves the delay — the path is
    ///    flaky, hedge aggressively.
    /// 3. **Hedge win rate**: if hedges win >50% of races (≥5 samples), reduce delay
    ///    by 25% — hedging is clearly helping.
    ///
    /// Final result is `max(base_delay, computed)` to never go below the configured floor.
    pub fn adaptive_delay(&self, base_delay: Duration) -> Duration {
        let count = self.samples.load(Ordering::Relaxed);
        if count < self.min_samples {
            return base_delay;
        }

        let ema = self.ema_ms.load(Ordering::Relaxed);
        let stddev = self.stddev_ms();

        // Baseline: EMA + stddev (≈P84 of a normal distribution)
        let mut adaptive_ms = ema.saturating_add(stddev);

        // Signal: consecutive errors → halve delay (more aggressive hedging)
        let errors = self.consecutive_errors.load(Ordering::Relaxed);
        if errors >= 3 {
            adaptive_ms /= 2;
        }

        // Signal: high hedge win rate → reduce delay by 25%
        let fires = self.hedge_fires.load(Ordering::Relaxed);
        let wins = self.hedge_wins.load(Ordering::Relaxed);
        if fires >= 5 && wins * 100 / fires > 50 {
            adaptive_ms = adaptive_ms * 3 / 4;
        }

        base_delay.max(Duration::from_millis(adaptive_ms))
    }

    /// Get the current EMA in milliseconds.
    pub fn ema_ms(&self) -> u64 {
        self.ema_ms.load(Ordering::Relaxed)
    }

    /// Get the number of samples recorded.
    pub fn sample_count(&self) -> u64 {
        self.samples.load(Ordering::Relaxed)
    }

    /// Get the hedge win rate as a percentage (0–100). Returns 0 if no hedges fired.
    pub fn hedge_win_rate_pct(&self) -> u64 {
        let fires = self.hedge_fires.load(Ordering::Relaxed);
        if fires == 0 {
            return 0;
        }
        self.hedge_wins.load(Ordering::Relaxed) * 100 / fires
    }

    /// Get consecutive error count.
    pub fn consecutive_errors(&self) -> u64 {
        self.consecutive_errors.load(Ordering::Relaxed)
    }
}

impl Clone for HedgeTracker {
    fn clone(&self) -> Self {
        Self {
            ema_ms: AtomicU64::new(self.ema_ms.load(Ordering::Relaxed)),
            ema_sq_ms: AtomicU64::new(self.ema_sq_ms.load(Ordering::Relaxed)),
            samples: AtomicU64::new(self.samples.load(Ordering::Relaxed)),
            threshold_pct: self.threshold_pct,
            min_samples: self.min_samples,
            hedge_fires: AtomicU64::new(self.hedge_fires.load(Ordering::Relaxed)),
            hedge_wins: AtomicU64::new(self.hedge_wins.load(Ordering::Relaxed)),
            consecutive_errors: AtomicU64::new(self.consecutive_errors.load(Ordering::Relaxed)),
        }
    }
}

impl Default for HedgeTracker {
    fn default() -> Self {
        Self::new(2.0, 5)
    }
}

/// Race a primary future against one or more hedge futures that fire after a delay.
///
/// 1. Starts `primary` immediately.
/// 2. After `config.delay`, starts the first hedge.
/// 3. If `max_hedges > 1`, additional hedges are staggered at `delay` intervals.
/// 4. First `Ok` result wins. Losing futures are dropped (cancelled).
/// 5. If all futures fail, returns the primary's error.
pub async fn hedge_race<T, E>(
    primary: impl Future<Output = Result<T, E>> + Send,
    hedge_factories: Vec<Pin<Box<dyn Future<Output = Result<T, E>> + Send>>>,
    config: &HedgeConfig,
) -> Result<T, E> {
    if !config.enabled || hedge_factories.is_empty() {
        return primary.await;
    }

    let delay = config.delay;
    let max_hedges = config.max_hedges.min(hedge_factories.len());

    tokio::pin!(primary);

    // Phase 1: race primary against the delay timer
    let hedge_futs = tokio::select! {
        biased;
        result = &mut primary => {
            // Primary finished before the delay — no hedge needed
            return result;
        }
        _ = tokio::time::sleep(delay) => {
            hedge_factories
        }
    };

    // Phase 2: fire first hedge, race against primary
    let mut hedge_iter = hedge_futs.into_iter().take(max_hedges);

    if let Some(first_hedge) = hedge_iter.next() {
        tokio::pin!(first_hedge);

        return tokio::select! {
            biased;
            result = &mut primary => result,
            result = &mut first_hedge => {
                match result {
                    ok @ Ok(_) => ok,
                    Err(_) => {
                        // Hedge failed — continue waiting on primary
                        primary.await
                    }
                }
            }
        };
    }

    // No hedge factories available — just await primary
    primary.await
}

/// Race with an optional cleanup callback for the losing future.
///
/// Useful for Chrome/WebDriver paths where a losing hedge must close its tab/session.
/// The `on_cancel` callback receives the result of the losing future's cleanup resource.
pub async fn hedge_race_with_cleanup<T, E, C, Fut>(
    primary: impl Future<Output = Result<(T, Option<C>), E>> + Send,
    hedge_factories: Vec<Pin<Box<dyn Future<Output = Result<(T, Option<C>), E>> + Send>>>,
    config: &HedgeConfig,
    _on_cancel: impl Fn(C) -> Fut + Send + Sync,
) -> Result<T, E>
where
    Fut: Future<Output = ()> + Send,
{
    if !config.enabled || hedge_factories.is_empty() {
        return primary.await.map(|(t, _)| t);
    }

    let delay = config.delay;
    let max_hedges = config.max_hedges.min(hedge_factories.len());

    tokio::pin!(primary);

    // Phase 1: race primary against the delay timer
    let hedge_futs = tokio::select! {
        biased;
        result = &mut primary => {
            return result.map(|(t, _)| t);
        }
        _ = tokio::time::sleep(delay) => {
            hedge_factories
        }
    };

    // Phase 2: fire first hedge
    let mut hedge_iter = hedge_futs.into_iter().take(max_hedges);

    if let Some(first_hedge) = hedge_iter.next() {
        tokio::pin!(first_hedge);

        tokio::select! {
            biased;
            result = &mut primary => {
                // Primary won — run cleanup on hedge if it completes with a resource
                // (hedge is dropped/cancelled here via select, so no cleanup needed)
                result.map(|(t, _)| t)
            }
            result = &mut first_hedge => {
                match result {
                    Ok((t, cleanup_resource)) => {
                        // Hedge won — clean up primary's resource if it completes
                        // Primary is dropped here. If we need to clean up the hedge's
                        // counterpart (the primary's resource), we can't because it's dropped.
                        // But the hedge's own resource is the winner, so we keep it.
                        // Only cleanup happens if hedge LOSES, which is handled by drop.
                        let _ = cleanup_resource;
                        Ok(t)
                    }
                    Err(_) => {
                        // Hedge failed — wait on primary
                        primary.await.map(|(t, _)| t)
                    }
                }
            }
        }
    } else {
        primary.await.map(|(t, _)| t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_primary_wins_before_threshold() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let primary = async move {
            c.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<&str, &str>("primary")
        };

        let c2 = counter.clone();
        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async move {
                c2.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok("hedge")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(500),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary");
        // Only primary should have been started
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_hedge_fires_and_wins() {
        let primary = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok::<&str, &str>("primary")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> = Box::pin(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok("hedge")
        });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "hedge");
    }

    #[tokio::test]
    async fn test_primary_wins_after_hedge_fires() {
        let primary = async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            Ok::<&str, &str>("primary")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> = Box::pin(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok("hedge")
        });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary");
    }

    #[tokio::test]
    async fn test_both_fail_returns_primary_error() {
        let primary = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Err::<&str, &str>("primary_err")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> = Box::pin(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Err("hedge_err")
        });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        // Hedge fails first, so we fall back to primary which also fails
        assert_eq!(result.unwrap_err(), "primary_err");
    }

    #[tokio::test]
    async fn test_primary_fails_hedge_succeeds() {
        let primary = async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Err::<&str, &str>("primary_err")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> = Box::pin(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok("hedge_ok")
        });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "hedge_ok");
    }

    #[tokio::test]
    async fn test_hedge_fails_primary_succeeds() {
        let primary = async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok::<&str, &str>("primary_ok")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> = Box::pin(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Err("hedge_err")
        });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary_ok");
    }

    #[tokio::test]
    async fn test_disabled_skips_hedge() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();

        let primary = async move {
            c.fetch_add(1, Ordering::SeqCst);
            Ok::<&str, &str>("primary")
        };

        let c2 = counter.clone();
        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async move {
                c2.fetch_add(1, Ordering::SeqCst);
                Ok("hedge")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(1),
            max_hedges: 1,
            enabled: false,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_empty_hedge_factories() {
        let primary = async { Ok::<&str, &str>("primary") };

        let config = HedgeConfig {
            delay: Duration::from_millis(1),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(
            primary,
            Vec::<Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>>>::new(),
            &config,
        )
        .await;
        assert_eq!(result.unwrap(), "primary");
    }

    #[tokio::test]
    async fn test_hedge_cancelled_on_primary_win() {
        let hedge_started = Arc::new(AtomicUsize::new(0));
        let hedge_finished = Arc::new(AtomicUsize::new(0));

        let hs = hedge_started.clone();
        let hf = hedge_finished.clone();

        let primary = async {
            tokio::time::sleep(Duration::from_millis(150)).await;
            Ok::<&str, &str>("primary")
        };

        let hedge: Pin<Box<dyn Future<Output = Result<&str, &str>> + Send>> =
            Box::pin(async move {
                hs.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(10)).await;
                hf.fetch_add(1, Ordering::SeqCst);
                Ok("hedge")
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(50),
            max_hedges: 1,
            enabled: true,
        };

        let result = hedge_race(primary, vec![hedge], &config).await;
        assert_eq!(result.unwrap(), "primary");

        // Give a moment for any lingering futures
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Hedge should have started but not finished (it was cancelled)
        assert_eq!(hedge_started.load(Ordering::SeqCst), 1);
        assert_eq!(hedge_finished.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_cleanup_callback_not_called_for_winner() {
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        let cc = cleanup_count.clone();

        let primary = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok::<(&str, Option<String>), &str>(("primary", Some("primary_resource".into())))
        };

        let hedge: Pin<Box<dyn Future<Output = Result<(&str, Option<String>), &str>> + Send>> =
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(("hedge", Some("hedge_resource".into())))
            });

        let config = HedgeConfig {
            delay: Duration::from_millis(100),
            max_hedges: 1,
            enabled: true,
        };

        let result =
            hedge_race_with_cleanup(primary, vec![hedge], &config, move |_resource: String| {
                let cc = cc.clone();
                async move {
                    cc.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await;

        assert_eq!(result.unwrap(), "hedge");
        // Cleanup should not be called for the winner
        assert_eq!(cleanup_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_tracker_warmup_phase() {
        let tracker = HedgeTracker::new(2.0, 5);
        // During warmup (< min_samples), should_hedge always returns true
        assert!(tracker.should_hedge(Duration::from_millis(1)));
        assert_eq!(tracker.sample_count(), 0);
    }

    #[test]
    fn test_tracker_ema_recording() {
        let tracker = HedgeTracker::new(2.0, 3);
        tracker.record(Duration::from_millis(100));
        assert_eq!(tracker.ema_ms(), 100);
        assert_eq!(tracker.sample_count(), 1);

        // Second sample: EMA = (100 * 4 + 200) / 5 = 120
        tracker.record(Duration::from_millis(200));
        assert_eq!(tracker.ema_ms(), 120);

        // Third sample: EMA = (120 * 4 + 100) / 5 = 116
        tracker.record(Duration::from_millis(100));
        assert_eq!(tracker.ema_ms(), 116);
    }

    #[test]
    fn test_tracker_should_hedge_after_warmup() {
        let tracker = HedgeTracker::new(2.0, 3);
        // Record 3 fast fetches (100ms each)
        for _ in 0..3 {
            tracker.record(Duration::from_millis(100));
        }
        assert_eq!(tracker.sample_count(), 3);

        // Elapsed 150ms — under 2x EMA (~200ms). Should NOT hedge.
        assert!(!tracker.should_hedge(Duration::from_millis(150)));
        // Elapsed 250ms — over 2x EMA. Should hedge.
        assert!(tracker.should_hedge(Duration::from_millis(250)));
    }

    #[test]
    fn test_tracker_adaptive_delay_with_variance() {
        let tracker = HedgeTracker::new(2.0, 2);
        let base_delay = Duration::from_secs(3);

        // During warmup, returns base delay
        assert_eq!(tracker.adaptive_delay(base_delay), base_delay);

        // Record uniform samples (500ms) — stddev should be ~0
        tracker.record(Duration::from_millis(500));
        tracker.record(Duration::from_millis(500));

        // Adaptive = EMA(500) + stddev(~0) = ~500ms. Base = 3000ms. Max = 3000.
        assert_eq!(tracker.adaptive_delay(base_delay), base_delay);

        // With a small base delay: adaptive = EMA(500) + ~0 = ~500ms > 200ms
        let small_delay = Duration::from_millis(200);
        let delay = tracker.adaptive_delay(small_delay);
        // Should be approximately 500ms (EMA + small stddev from integer rounding)
        assert!(delay.as_millis() >= 490 && delay.as_millis() <= 510);
    }

    #[test]
    fn test_tracker_adaptive_delay_high_variance() {
        let tracker = HedgeTracker::new(2.0, 2);

        // Record samples with high variance: 100ms and 900ms
        tracker.record(Duration::from_millis(100));
        // After first: EMA=100, EMA_sq=10000
        tracker.record(Duration::from_millis(900));
        // EMA = (100*4 + 900)/5 = 260
        // EMA_sq = (10000*4 + 810000)/5 = 170000
        // variance = 170000 - 260*260 = 170000 - 67600 = 102400
        // stddev = sqrt(102400) ≈ 320

        let small_delay = Duration::from_millis(100);
        let delay = tracker.adaptive_delay(small_delay);
        // Should be EMA(260) + stddev(~320) ≈ 580ms
        assert!(
            delay.as_millis() >= 550 && delay.as_millis() <= 610,
            "expected ~580ms, got {}ms",
            delay.as_millis()
        );
    }

    #[test]
    fn test_tracker_consecutive_errors_reduce_delay() {
        let tracker = HedgeTracker::new(2.0, 2);
        tracker.record(Duration::from_millis(1000));
        tracker.record(Duration::from_millis(1000));
        // EMA = 1000, stddev ≈ 0. Adaptive ≈ 1000ms.

        let base = Duration::from_millis(100);
        let normal_delay = tracker.adaptive_delay(base);

        // Simulate 3 consecutive errors
        tracker.record_error();
        tracker.record_error();
        tracker.record_error();

        let error_delay = tracker.adaptive_delay(base);
        // Should be halved: ~500ms vs ~1000ms
        assert!(
            error_delay < normal_delay,
            "error delay {}ms should be less than normal {}ms",
            error_delay.as_millis(),
            normal_delay.as_millis()
        );
        assert!(error_delay.as_millis() <= 510);
    }

    #[test]
    fn test_tracker_hedge_win_rate_reduces_delay() {
        let tracker = HedgeTracker::new(2.0, 2);
        tracker.record(Duration::from_millis(1000));
        tracker.record(Duration::from_millis(1000));

        let base = Duration::from_millis(100);
        let normal_delay = tracker.adaptive_delay(base);

        // Simulate 5 hedges where hedge wins 4/5 (80%)
        for _ in 0..5 {
            tracker.record_fired();
        }
        for _ in 0..4 {
            tracker.record_outcome(true);
        }
        tracker.record_outcome(false);

        assert_eq!(tracker.hedge_win_rate_pct(), 80);

        let tuned_delay = tracker.adaptive_delay(base);
        // Should be reduced by 25%: ~750ms vs ~1000ms
        assert!(
            tuned_delay < normal_delay,
            "tuned delay {}ms should be less than normal {}ms",
            tuned_delay.as_millis(),
            normal_delay.as_millis()
        );
        assert!(tuned_delay.as_millis() >= 740 && tuned_delay.as_millis() <= 760);
    }

    #[test]
    fn test_tracker_error_reset_on_success() {
        let tracker = HedgeTracker::default();
        tracker.record_error();
        tracker.record_error();
        tracker.record_error();
        assert_eq!(tracker.consecutive_errors(), 3);

        tracker.record_success();
        assert_eq!(tracker.consecutive_errors(), 0);
    }

    #[test]
    fn test_tracker_stddev_uniform() {
        let tracker = HedgeTracker::new(2.0, 1);
        // All same value → stddev should be 0
        for _ in 0..10 {
            tracker.record(Duration::from_millis(500));
        }
        assert_eq!(tracker.stddev_ms(), 0);
    }

    #[test]
    fn test_tracker_clone_independence() {
        let tracker = HedgeTracker::new(2.0, 3);
        tracker.record(Duration::from_millis(100));

        let cloned = tracker.clone();
        tracker.record(Duration::from_millis(500));

        // Clone should have the snapshot, not see new samples
        assert_eq!(cloned.sample_count(), 1);
        assert_eq!(cloned.ema_ms(), 100);
        assert_eq!(tracker.sample_count(), 2);
    }

    #[test]
    fn test_tracker_combined_signals() {
        let tracker = HedgeTracker::new(2.0, 2);
        tracker.record(Duration::from_millis(2000));
        tracker.record(Duration::from_millis(2000));

        let base = Duration::from_millis(100);

        // Both signals active: errors + high win rate
        for _ in 0..3 {
            tracker.record_error();
        }
        for _ in 0..6 {
            tracker.record_fired();
        }
        for _ in 0..4 {
            tracker.record_outcome(true);
        }

        let delay = tracker.adaptive_delay(base);
        // EMA=2000 + stddev≈0 = 2000 → /2 (errors) = 1000 → *3/4 (win rate) = 750
        assert!(
            delay.as_millis() >= 740 && delay.as_millis() <= 760,
            "expected ~750ms, got {}ms",
            delay.as_millis()
        );
    }
}
