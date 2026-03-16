//! AIMD-based adaptive concurrency controller.
//!
//! Uses Additive Increase / Multiplicative Decrease to dynamically tune
//! the concurrency limit based on observed success and failure signals.
//! All operations are lock-free (`AtomicUsize` with `Relaxed` ordering).

use std::sync::atomic::{AtomicUsize, Ordering};

/// AIMD (Additive Increase / Multiplicative Decrease) concurrency controller.
///
/// On sustained success the limit grows linearly (+1 per `increase_threshold`
/// consecutive successes). On failure the limit is halved immediately,
/// clamped to `min_limit`.
///
/// Thread-safe and entirely lock-free.
pub struct AIMDController {
    current_limit: AtomicUsize,
    min_limit: usize,
    max_limit: usize,
    success_count: AtomicUsize,
    increase_threshold: usize,
    /// Stored as a right-shift amount. `decrease_factor = 0.5` → `shift = 1`.
    decrease_shift: u32,
}

impl AIMDController {
    /// Create a new controller.
    ///
    /// * `initial_limit` – starting concurrency target (clamped to `[min, max]`).
    /// * `min_limit` – floor for the concurrency limit (must be ≥ 1).
    /// * `max_limit` – ceiling for the concurrency limit.
    /// * `increase_threshold` – number of consecutive successes before additive
    ///   increase (default: 10).
    /// * `decrease_factor` – multiplicative decrease factor (default: 0.5).
    ///   Internally stored as a right-shift; only 0.5 is currently supported as
    ///   an exact power-of-two shift.
    pub fn new(
        initial_limit: usize,
        min_limit: usize,
        max_limit: usize,
        increase_threshold: usize,
        _decrease_factor: f64,
    ) -> Self {
        let min_limit = min_limit.max(1);
        let max_limit = max_limit.max(min_limit);
        let initial = initial_limit.clamp(min_limit, max_limit);
        let threshold = if increase_threshold == 0 {
            10
        } else {
            increase_threshold
        };

        Self {
            current_limit: AtomicUsize::new(initial),
            min_limit,
            max_limit,
            success_count: AtomicUsize::new(0),
            increase_threshold: threshold,
            decrease_shift: 1, // 0.5 → >> 1
        }
    }

    /// Create a controller with sensible defaults.
    ///
    /// `initial` is clamped to `[1, max_limit]`, threshold = 10, factor = 0.5.
    pub fn with_defaults(initial: usize, max_limit: usize) -> Self {
        Self::new(initial, 1, max_limit, 10, 0.5)
    }

    /// Record a successful request.
    ///
    /// After `increase_threshold` consecutive successes the limit grows by 1
    /// (up to `max_limit`) and the counter resets.
    pub fn record_success(&self) {
        let prev = self.success_count.fetch_add(1, Ordering::Relaxed);
        // `prev` is the value *before* the add, so `prev + 1` is the new count.
        if prev.saturating_add(1) >= self.increase_threshold {
            self.success_count.store(0, Ordering::Relaxed);
            // Additive increase — try to bump by 1 if below max.
            let _ = self
                .current_limit
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
                    if cur < self.max_limit {
                        Some(cur.saturating_add(1))
                    } else {
                        None
                    }
                });
        }
    }

    /// Record a failed request.
    ///
    /// Immediately halves the current limit (clamped to `min_limit`) and resets
    /// the success counter.
    pub fn record_failure(&self) {
        self.success_count.store(0, Ordering::Relaxed);
        let _ = self
            .current_limit
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| {
                let halved = cur >> self.decrease_shift;
                let next = halved.max(self.min_limit);
                if next != cur {
                    Some(next)
                } else {
                    None
                }
            });
    }

    /// Current concurrency target.
    #[inline]
    pub fn current_limit(&self) -> usize {
        self.current_limit.load(Ordering::Relaxed)
    }

    /// Minimum configured limit.
    #[inline]
    pub fn min_limit(&self) -> usize {
        self.min_limit
    }

    /// Maximum configured limit.
    #[inline]
    pub fn max_limit(&self) -> usize {
        self.max_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_clamp() {
        let c = AIMDController::new(100, 2, 10, 10, 0.5);
        assert_eq!(c.current_limit(), 10);

        let c = AIMDController::new(0, 3, 10, 10, 0.5);
        assert_eq!(c.current_limit(), 3);
    }

    #[test]
    fn additive_increase_after_threshold() {
        let c = AIMDController::new(5, 1, 100, 5, 0.5);
        assert_eq!(c.current_limit(), 5);

        for _ in 0..5 {
            c.record_success();
        }
        assert_eq!(c.current_limit(), 6);

        for _ in 0..5 {
            c.record_success();
        }
        assert_eq!(c.current_limit(), 7);
    }

    #[test]
    fn increase_capped_at_max() {
        let c = AIMDController::new(9, 1, 10, 1, 0.5);
        c.record_success(); // 9 → 10
        assert_eq!(c.current_limit(), 10);
        c.record_success(); // stays 10
        assert_eq!(c.current_limit(), 10);
    }

    #[test]
    fn multiplicative_decrease() {
        let c = AIMDController::new(20, 1, 100, 10, 0.5);
        c.record_failure();
        assert_eq!(c.current_limit(), 10);
        c.record_failure();
        assert_eq!(c.current_limit(), 5);
    }

    #[test]
    fn decrease_clamped_to_min() {
        let c = AIMDController::new(4, 3, 100, 10, 0.5);
        c.record_failure(); // 4 >> 1 = 2, clamped to 3
        assert_eq!(c.current_limit(), 3);
        c.record_failure(); // 3 >> 1 = 1, clamped to 3
        assert_eq!(c.current_limit(), 3);
    }

    #[test]
    fn failure_resets_success_counter() {
        let c = AIMDController::new(10, 1, 100, 5, 0.5);
        // Accumulate 4 successes (one short of threshold)
        for _ in 0..4 {
            c.record_success();
        }
        c.record_failure(); // resets counter, halves limit
        assert_eq!(c.current_limit(), 5);

        // One more success should NOT trigger increase (counter was reset)
        c.record_success();
        assert_eq!(c.current_limit(), 5);
    }

    #[test]
    fn with_defaults_constructor() {
        let c = AIMDController::with_defaults(8, 50);
        assert_eq!(c.current_limit(), 8);
        assert_eq!(c.min_limit(), 1);
        assert_eq!(c.max_limit(), 50);
    }

    #[test]
    fn min_greater_than_max_corrected() {
        let c = AIMDController::new(5, 20, 10, 10, 0.5);
        // max is corrected to max(10, 20) = 20
        assert_eq!(c.max_limit(), 20);
        assert_eq!(c.min_limit(), 20);
        assert_eq!(c.current_limit(), 20);
    }
}
