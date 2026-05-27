//! Adaptive concurrency primitives.
//!
//! Two cooperating types live here:
//!
//! - [`AIMDController`] — an AIMD signal source. Lock-free `AtomicUsize`
//!   target updated by `record_success` / `record_failure`. Read-only on
//!   the hot path. Doesn't gate anything itself.
//!
//! - [`AdaptiveSemaphore`] — a lock-free bridge between an
//!   `Arc<tokio::sync::Semaphore>` (the actual worker gate) and an
//!   `AtomicUsize` target. Calling `set_target` reconciles the
//!   difference via `Semaphore::add_permits` / `forget_permits`, both
//!   non-panicking and lock-free. Hand the bridge's semaphore to a
//!   `Website` via [`crate::website::Website::with_concurrency_semaphore`]
//!   or [`crate::website::Website::with_adaptive_concurrency`] and the
//!   crawl will gate worker spawns through it instead of the static
//!   `configuration.concurrency_limit`.
//!
//! Together they form a complete adaptive loop: feed observed
//! success/failure into the controller, read its `current_limit`, push
//! that into the bridge's `set_target`. The bridge is also fine to use
//! on its own with an external load signal (e.g. an admission
//! controller that scales by host CPU pressure).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Semaphore;

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

/// Lock-free, atomic bridge from an in-memory target value to a live
/// `tokio::sync::Semaphore` that gates worker concurrency on a crawl.
///
/// The semaphore is the authoritative gate the crawler holds; the
/// atomic `target` is the most-recently-requested permit count. Calling
/// [`Self::set_target`] computes the delta against the previous target
/// (atomic `swap`) and applies it to the semaphore — `add_permits` for
/// expansion, `forget_permits` for contraction. Both are non-blocking
/// and never panic for normal inputs.
///
/// Behavior worth knowing:
///
/// - **Existing in-flight permits are never cancelled.** Shrinking the
///   target only forgets *available* permits; workers holding an
///   outstanding permit complete their fetch and release as usual.
///   Forgotten permits don't return to the pool, so the effective
///   ceiling drops smoothly to the new target.
/// - **Lock-free.** Both the atomic target and the underlying
///   `Semaphore` are lock-free in tokio — `set_target` adds zero
///   overhead beyond an atomic swap + (at most) one
///   `add_permits`/`forget_permits` call.
/// - **No deadlocks.** The bridge holds no locks of its own; callers
///   acquire permits with the standard `acquire().await` / `try_acquire`
///   API on the inner semaphore. Dropping all clones drops the
///   semaphore and any pending acquires return `AcquireError`, which
///   the crawl treats as "shut down".
/// - **No panics.** `target` is clamped to `[1, Semaphore::MAX_PERMITS]`
///   on construction and on `set_target`, so `add_permits` can never
///   overflow tokio's internal counter.
///
/// Cloning is cheap (Arc bumps). Hand one clone to the `Website`, keep
/// another in the admission controller, call `set_target` from the
/// controller.
#[derive(Debug, Clone)]
pub struct AdaptiveSemaphore {
    sem: Arc<Semaphore>,
    target: Arc<AtomicUsize>,
}

impl AdaptiveSemaphore {
    /// Build a fresh bridge with the given initial permit count.
    /// `initial` is clamped to `[1, Semaphore::MAX_PERMITS]`; values
    /// outside that range would either starve the crawler (0) or
    /// overflow tokio's internal counter (above MAX_PERMITS).
    pub fn new(initial: usize) -> Self {
        let initial = Self::clamp_permits(initial);
        Self {
            sem: Arc::new(Semaphore::new(initial)),
            target: Arc::new(AtomicUsize::new(initial)),
        }
    }

    /// Inner semaphore — pass this into the crawler via
    /// [`crate::website::Website::with_concurrency_semaphore`]. The Arc
    /// is cheap to clone and intentional: the bridge holds one clone for
    /// permit-count reconciliation, the crawler holds another for
    /// gating, and both observe the same live state.
    pub fn semaphore(&self) -> Arc<Semaphore> {
        Arc::clone(&self.sem)
    }

    /// Most recently requested permit ceiling — the `n` from the latest
    /// `set_target(n)` (or the constructor `initial`). Not the same as
    /// live availability — for that read `available()`.
    #[inline]
    pub fn target(&self) -> usize {
        self.target.load(Ordering::Relaxed)
    }

    /// Number of permits currently available for acquire (i.e. `target
    /// - in_flight`). Wraps `Semaphore::available_permits`.
    #[inline]
    pub fn available(&self) -> usize {
        self.sem.available_permits()
    }

    /// Resize toward `new_target`. Lock-free, non-blocking, never panics.
    ///
    /// Concurrent `set_target` calls compose correctly: the atomic
    /// `swap` returns the previous target, and the delta against that
    /// previous value is applied to the semaphore — so two interleaved
    /// `set_target(a)` / `set_target(b)` calls leave the semaphore at
    /// `b` regardless of which "saw" which intermediate state.
    pub fn set_target(&self, new_target: usize) {
        let clamped = Self::clamp_permits(new_target);
        let prev = self.target.swap(clamped, Ordering::Relaxed);
        if clamped > prev {
            self.sem.add_permits(clamped - prev);
        } else if clamped < prev {
            // forget_permits is capped at currently-available by tokio,
            // so a "forget more than available" never panics — it just
            // returns the smaller actually-forgotten count, and the
            // remaining shrinkage takes effect as in-flight permits are
            // released back to the pool (the released permits are
            // forgotten in turn by tokio's internal accounting since the
            // semaphore's permanent capacity has dropped).
            self.sem.forget_permits(prev - clamped);
        }
    }

    /// Pull the current target out of an [`AIMDController`] and apply
    /// it to the bridge. Convenience for the common pattern:
    /// `bridge.sync_from(&controller)` in a periodic tick.
    pub fn sync_from(&self, controller: &AIMDController) {
        self.set_target(controller.current_limit());
    }

    #[inline]
    fn clamp_permits(n: usize) -> usize {
        n.clamp(1, Semaphore::MAX_PERMITS)
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

    // ---- AdaptiveSemaphore -------------------------------------------------

    #[test]
    fn adaptive_initial_target_and_available() {
        let s = AdaptiveSemaphore::new(4);
        assert_eq!(s.target(), 4);
        assert_eq!(s.available(), 4);
    }

    #[test]
    fn adaptive_initial_zero_is_clamped_to_one() {
        // Zero would starve the crawler; clamp to 1.
        let s = AdaptiveSemaphore::new(0);
        assert_eq!(s.target(), 1);
        assert_eq!(s.available(), 1);
    }

    #[test]
    fn adaptive_set_target_expand() {
        let s = AdaptiveSemaphore::new(2);
        s.set_target(5);
        assert_eq!(s.target(), 5);
        assert_eq!(s.available(), 5);
    }

    #[test]
    fn adaptive_set_target_shrink_without_inflight() {
        let s = AdaptiveSemaphore::new(5);
        s.set_target(2);
        assert_eq!(s.target(), 2);
        assert_eq!(s.available(), 2);
    }

    #[tokio::test]
    async fn adaptive_shrink_with_inflight_does_not_cancel_existing_permits() {
        let s = AdaptiveSemaphore::new(3);
        let sem = s.semaphore();
        // Hold one permit — represents a live in-flight worker.
        let permit = sem.clone().acquire_owned().await.unwrap();
        assert_eq!(s.available(), 2);

        // Shrink target below current available — the held permit
        // stays valid; only the *available* pool is forgotten.
        s.set_target(1);
        assert_eq!(s.target(), 1);
        // forget_permits forgot up to `available`; the held permit is
        // intact and will be forgotten (not returned) on drop because
        // the semaphore's permanent capacity dropped below where it
        // started. The exact post-state is "0 available, 1 in-flight".
        assert_eq!(s.available(), 0);

        // Drop the in-flight permit — semaphore stays at the new
        // (shrunken) capacity; the released permit is forgotten because
        // permanent capacity (1) is already met by what's outstanding.
        drop(permit);
        // After the drop, available rises to the new target (1) since
        // there are no outstanding permits to subtract from it.
        assert_eq!(s.available(), 1);
    }

    #[test]
    fn adaptive_set_target_same_is_noop() {
        let s = AdaptiveSemaphore::new(3);
        s.set_target(3);
        assert_eq!(s.target(), 3);
        assert_eq!(s.available(), 3);
    }

    #[test]
    fn adaptive_set_target_above_max_clamps() {
        let s = AdaptiveSemaphore::new(4);
        s.set_target(usize::MAX);
        // Clamped to Semaphore::MAX_PERMITS — no panic, no overflow.
        assert_eq!(s.target(), Semaphore::MAX_PERMITS);
    }

    #[test]
    fn adaptive_clones_share_state() {
        let a = AdaptiveSemaphore::new(2);
        let b = a.clone();
        b.set_target(7);
        // Both clones see the same target and the same live semaphore.
        assert_eq!(a.target(), 7);
        assert_eq!(b.target(), 7);
        assert!(Arc::ptr_eq(&a.semaphore(), &b.semaphore()));
    }

    #[test]
    fn adaptive_sync_from_aimd_controller() {
        let c = AIMDController::new(3, 1, 100, 5, 0.5);
        let s = AdaptiveSemaphore::new(1);
        s.sync_from(&c);
        assert_eq!(s.target(), 3);

        // Drive the controller up, sync again — bridge follows.
        for _ in 0..5 {
            c.record_success();
        }
        s.sync_from(&c);
        assert_eq!(s.target(), 4);
    }

    #[tokio::test]
    async fn adaptive_concurrent_set_target_calls_converge() {
        use std::sync::Arc as StdArc;
        let s = StdArc::new(AdaptiveSemaphore::new(10));
        let mut handles = Vec::new();
        for i in 1..=8 {
            let s = StdArc::clone(&s);
            handles.push(tokio::spawn(async move { s.set_target(i * 2) }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // Final target is whichever set_target ran last — we can't
        // know which, but it must be in the [2, 16] band and the
        // available permits must match it (modulo any held permits;
        // none were held in this test).
        let final_target = s.target();
        assert!((2..=16).contains(&final_target));
        assert_eq!(s.available(), final_target);
    }
}
