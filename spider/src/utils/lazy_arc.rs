//! Lock-free, refcount-dropped lazy `Arc<T>`.
//!
//! [`LazyArc<T>`] holds only a `Weak<T>` and rebuilds the value on
//! demand whenever the weak link is dead. Each caller of
//! [`LazyArc::get_or_build`] receives an `Arc<T>`; the value lives
//! exactly as long as some clone of that `Arc` is alive somewhere. When
//! the last clone is dropped, the inner `T` is freed automatically —
//! no GC, no idle timer, no explicit release call.
//!
//! This is the load-bearing primitive behind spider's secondary HTTP
//! client: a request that wants a non-default proxy upgrades or builds
//! an `Arc<Client>` just for itself; concurrent peers share the same
//! one; once they all finish, the connection pool tied to that client
//! drops with it.
//!
//! # Concurrency model
//!
//! * **Reads (hot path):** one atomic load + `Weak::upgrade`. No
//!   syscall, no allocation, no contention beyond the atomic itself.
//! * **Cold miss (Weak dead):** one allocation for the new `Arc`,
//!   one atomic store for the new `Weak`. Multiple threads racing the
//!   same cold miss may each construct their own value — every caller
//!   gets a working `Arc`, but only the last writer's `Weak` is
//!   cached. The losers' fresh `Arc`s remain valid for their owners
//!   and drop normally on scope exit.
//! * **No `Mutex` and no `await`:** all operations complete in finite
//!   atomic steps. Deadlock is structurally impossible.
//!
//! # Why `Weak`?
//!
//! Holding a strong `Arc` would pin the value forever. Holding a `Weak`
//! lets the value tie its lifetime to active borrowers. The cost of an
//! `upgrade` failure (i.e. rebuild on next access) is paid only after
//! the last borrower has finished, which is exactly when rebuilding is
//! cheap (no contention, idle path).
//!
//! # Examples
//!
//! ```
//! use spider::utils::lazy_arc::LazyArc;
//! use std::sync::Arc;
//!
//! let lazy: LazyArc<String> = LazyArc::new();
//! let a = lazy.get_or_build(|| String::from("expensive value"));
//! let b = lazy.get_or_build(|| panic!("should not rebuild"));
//! assert!(Arc::ptr_eq(&a, &b));
//! drop(a);
//! drop(b);
//! // Both clones gone — next get_or_build will rebuild.
//! let c = lazy.get_or_build(|| String::from("rebuilt"));
//! assert_eq!(&*c, "rebuilt");
//! ```

use arc_swap::ArcSwap;
use std::sync::{Arc, Weak};

/// A lock-free, lazy, refcount-dropped `Arc<T>` slot.
///
/// See the [module docs](self) for the concurrency model.
pub struct LazyArc<T> {
    /// The cached weak pointer to the most-recently-built value.
    /// Stored inside an `ArcSwap<Weak<T>>` so the swap on rebuild is
    /// atomic and never blocks. The outer `Arc<Weak<T>>` is just the
    /// container `arc_swap` requires; the meaningful state is the
    /// inner `Weak<T>`.
    weak: ArcSwap<Weak<T>>,
}

impl<T> LazyArc<T> {
    /// Build an empty slot. The first [`get_or_build`](Self::get_or_build)
    /// call will invoke its builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            weak: ArcSwap::from(Arc::new(Weak::new())),
        }
    }

    /// Return a live `Arc<T>`, building one via `build` if the cached
    /// weak link is dead.
    ///
    /// `build` is called at most once per "generation" (the period
    /// between rebuilds). Concurrent callers racing the same cold miss
    /// may each invoke `build`; every caller receives a usable `Arc`
    /// but only the last writer's pointer is cached. This is a benign
    /// race — `build` should be idempotent in the sense that any of
    /// its outputs is correct, even if not byte-identical.
    #[inline]
    pub fn get_or_build<F>(&self, build: F) -> Arc<T>
    where
        F: FnOnce() -> T,
    {
        // Hot path: cached weak still has a strong owner alive somewhere.
        if let Some(arc) = self.weak.load().upgrade() {
            return arc;
        }
        // Cold path: weak is dead, rebuild. The store may race with
        // peers — see the type-level docs for why that's fine.
        let arc = Arc::new(build());
        self.weak.store(Arc::new(Arc::downgrade(&arc)));
        arc
    }

    /// Try to upgrade the cached weak pointer without building.
    ///
    /// Returns `Some(arc)` when at least one strong owner is still alive,
    /// or `None` when the slot is dead (next [`get_or_build`](Self::get_or_build)
    /// will rebuild). Useful for "peek" style observation; never blocks.
    #[inline]
    pub fn try_get(&self) -> Option<Arc<T>> {
        self.weak.load().upgrade()
    }

    /// Whether the cached weak pointer currently has at least one strong
    /// owner alive. False positives are possible under concurrent drop
    /// (the slot may go dead between the check and any subsequent use);
    /// callers that need a usable `Arc` should call
    /// [`try_get`](Self::try_get) or [`get_or_build`](Self::get_or_build)
    /// directly.
    #[inline]
    pub fn is_live(&self) -> bool {
        self.weak.load().strong_count() > 0
    }
}

impl<T> Default for LazyArc<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> std::fmt::Debug for LazyArc<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyArc")
            .field("live", &self.is_live())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn first_call_invokes_builder() {
        let calls = AtomicUsize::new(0);
        let lazy: LazyArc<u64> = LazyArc::new();
        let a = lazy.get_or_build(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            42
        });
        assert_eq!(*a, 42);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn second_call_reuses_while_first_arc_lives() {
        let calls = AtomicUsize::new(0);
        let lazy: LazyArc<u64> = LazyArc::new();
        let a = lazy.get_or_build(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            42
        });
        let b = lazy.get_or_build(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            999
        });
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(*b, 42);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn rebuilds_after_all_clones_drop() {
        let calls = AtomicUsize::new(0);
        let lazy: LazyArc<u64> = LazyArc::new();
        {
            let a = lazy.get_or_build(|| {
                calls.fetch_add(1, Ordering::Relaxed);
                1
            });
            let b = lazy.get_or_build(|| {
                calls.fetch_add(1, Ordering::Relaxed);
                2
            });
            assert!(Arc::ptr_eq(&a, &b));
            assert!(lazy.is_live());
        }
        assert!(!lazy.is_live(), "expected slot dead after clones drop");

        let c = lazy.get_or_build(|| {
            calls.fetch_add(1, Ordering::Relaxed);
            7
        });
        assert_eq!(*c, 7);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn try_get_does_not_build() {
        let lazy: LazyArc<u64> = LazyArc::new();
        assert!(lazy.try_get().is_none());

        let _a = lazy.get_or_build(|| 5);
        assert!(lazy.try_get().is_some());
    }

    #[test]
    fn concurrent_cold_miss_all_callers_succeed() {
        // N threads racing the same cold miss must all produce a
        // usable Arc; the build closure is allowed to run more than
        // once but every output is acceptable.
        let lazy: Arc<LazyArc<u64>> = Arc::new(LazyArc::new());
        let total_builds = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..16)
            .map(|_| {
                let lazy = lazy.clone();
                let total_builds = total_builds.clone();
                std::thread::spawn(move || {
                    lazy.get_or_build(|| {
                        total_builds.fetch_add(1, Ordering::Relaxed);
                        // Tiny sleep to widen the race window.
                        std::thread::sleep(std::time::Duration::from_micros(50));
                        100
                    })
                })
            })
            .collect();

        for h in handles {
            let arc = h.join().unwrap();
            assert_eq!(*arc, 100);
        }

        // build_count is bounded by real concurrency — at least 1, at
        // most 16. The exact count is a property of the scheduler.
        let n = total_builds.load(Ordering::Relaxed);
        assert!((1..=16).contains(&n), "unexpected build count: {n}");
    }

    #[test]
    fn debug_impl_does_not_panic() {
        let lazy: LazyArc<u64> = LazyArc::new();
        let _ = format!("{:?}", lazy);
        let _arc = lazy.get_or_build(|| 1);
        let _ = format!("{:?}", lazy);
    }
}
