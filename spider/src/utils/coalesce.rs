//! Request coalescing to deduplicate concurrent in-flight requests for the same URL.
//!
//! Feature-gated behind `request_coalesce`. When multiple tasks request the
//! same URL concurrently, only one performs the actual fetch; the others wait
//! for completion and then read from cache or shared state.

#[cfg(feature = "request_coalesce")]
mod inner {
    use crate::compact_str::CompactString;
    use dashmap::DashMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tokio::sync::broadcast;

    /// Broadcast channel capacity — only one completion signal is sent per URL;
    /// a small buffer suffices for all subscribers.
    const CHANNEL_CAPACITY: usize = 4;

    /// Maximum time a guard can be held before it is considered stale (seconds).
    const STALE_TIMEOUT_SECS: u64 = 120;

    /// Result of calling [`RequestCoalescer::try_start`].
    pub enum CoalesceResult {
        /// This caller should perform the fetch. Call [`CoalesceGuard::complete`]
        /// (or just drop the guard) when done.
        Proceed(CoalesceGuard),
        /// Another caller is already fetching this URL. Await the receiver
        /// to be notified on completion, then read from cache/shared state.
        Wait(broadcast::Receiver<()>),
    }

    /// RAII guard that removes the in-flight entry on drop.
    ///
    /// The entry is removed when:
    /// - `complete()` is called explicitly, or
    /// - the guard is dropped (e.g., on panic or early return).
    ///
    /// This ensures no dangling entries even if the fetch task panics.
    pub struct CoalesceGuard {
        url: CompactString,
        in_flight: Arc<DashMap<CompactString, InFlightEntry>>,
        generation: Arc<()>,
        completed: bool,
    }

    struct InFlightEntry {
        sender: broadcast::Sender<()>,
        generation: Arc<()>,
        created_at: Instant,
    }

    impl CoalesceGuard {
        /// Signal that the fetch is complete and notify all waiters.
        pub fn complete(mut self) {
            self.finish();
        }

        fn finish(&mut self) {
            if self.completed {
                return;
            }
            self.completed = true;

            if let Some((_, entry)) = self.in_flight.remove_if(&self.url, |_, entry| {
                Arc::ptr_eq(&entry.generation, &self.generation)
            }) {
                let _ = entry.sender.send(());
            }
        }
    }

    impl Drop for CoalesceGuard {
        fn drop(&mut self) {
            self.finish();
        }
    }

    /// Deduplicates concurrent requests for the same URL.
    ///
    /// Thread-safe through short-lived DashMap shard locks. No guard is held
    /// across an await or a second map operation.
    pub struct RequestCoalescer {
        in_flight: Arc<DashMap<CompactString, InFlightEntry>>,
    }

    impl RequestCoalescer {
        /// Create a new coalescer.
        pub fn new() -> Self {
            Self {
                in_flight: Arc::new(DashMap::with_capacity(64)),
            }
        }

        /// Try to start a fetch for `url`.
        ///
        /// - Returns `CoalesceResult::Proceed(guard)` if no other task is fetching this URL.
        ///   The caller must perform the fetch and then call `guard.complete()`.
        /// - Returns `CoalesceResult::Wait(receiver)` if another task is already fetching.
        ///   The caller should `receiver.recv().await` and then read from cache.
        pub fn try_start(&self, url: &str) -> CoalesceResult {
            let key = CompactString::new(url);

            if let Some(entry) = self.in_flight.get(&key) {
                if entry.created_at.elapsed() < Duration::from_secs(STALE_TIMEOUT_SECS) {
                    return CoalesceResult::Wait(entry.sender.subscribe());
                }
            }

            use dashmap::mapref::entry::Entry;
            // Decide ownership while holding one shard lock. A read followed by
            // insert lets concurrent misses both become the fetch owner.
            let generation = Arc::new(());
            match self.in_flight.entry(key.clone()) {
                Entry::Occupied(mut entry) => {
                    if entry.get().created_at.elapsed() < Duration::from_secs(STALE_TIMEOUT_SECS) {
                        return CoalesceResult::Wait(entry.get().sender.subscribe());
                    }
                    let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
                    entry.insert(InFlightEntry {
                        sender,
                        generation: generation.clone(),
                        created_at: Instant::now(),
                    });
                }
                Entry::Vacant(entry) => {
                    let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
                    entry.insert(InFlightEntry {
                        sender,
                        generation: generation.clone(),
                        created_at: Instant::now(),
                    });
                }
            };

            CoalesceResult::Proceed(CoalesceGuard {
                url: key,
                in_flight: Arc::clone(&self.in_flight),
                generation,
                completed: false,
            })
        }

        /// Number of URLs currently in-flight.
        pub fn in_flight_count(&self) -> usize {
            self.in_flight.len()
        }

        /// Purge entries older than the stale timeout.
        /// Call periodically if long-running guards are a concern.
        pub fn purge_stale(&self) {
            let cutoff = Duration::from_secs(STALE_TIMEOUT_SECS);
            self.in_flight
                .retain(|_, entry| entry.created_at.elapsed() < cutoff);
        }
    }

    impl Default for RequestCoalescer {
        fn default() -> Self {
            Self::new()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn simultaneous_misses_have_one_owner() {
            let coalescer = Arc::new(RequestCoalescer::new());
            let barrier = Arc::new(std::sync::Barrier::new(16));
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    let c = coalescer.clone();
                    let b = barrier.clone();
                    std::thread::spawn(move || {
                        b.wait();
                        c.try_start("same-url")
                    })
                })
                .collect();
            // Keep owners alive until all contenders have returned.
            let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            assert_eq!(
                results
                    .iter()
                    .filter(|r| matches!(r, CoalesceResult::Proceed(_)))
                    .count(),
                1
            );
        }

        #[test]
        fn stale_owner_cannot_remove_replacement() {
            let c = RequestCoalescer::new();
            let old = match c.try_start("url") {
                CoalesceResult::Proceed(g) => g,
                _ => unreachable!(),
            };
            c.in_flight.get_mut("url").unwrap().created_at =
                Instant::now() - Duration::from_secs(STALE_TIMEOUT_SECS + 1);
            let new = match c.try_start("url") {
                CoalesceResult::Proceed(g) => g,
                _ => unreachable!(),
            };
            drop(old);
            assert_eq!(c.in_flight_count(), 1);
            let mut waiter = match c.try_start("url") {
                CoalesceResult::Wait(rx) => rx,
                _ => unreachable!(),
            };
            drop(new);
            assert!(waiter.try_recv().is_ok());
            assert_eq!(c.in_flight_count(), 0);
        }

        #[test]
        fn test_first_caller_proceeds() {
            let coalescer = RequestCoalescer::new();
            let _guard = match coalescer.try_start("https://example.com") {
                CoalesceResult::Proceed(g) => g,
                CoalesceResult::Wait(_) => panic!("first caller should proceed"),
            };
            assert_eq!(coalescer.in_flight_count(), 1);
        }

        #[test]
        fn test_second_caller_waits() {
            let coalescer = RequestCoalescer::new();
            let _guard = match coalescer.try_start("https://example.com") {
                CoalesceResult::Proceed(g) => g,
                CoalesceResult::Wait(_) => panic!("first caller should proceed"),
            };

            match coalescer.try_start("https://example.com") {
                CoalesceResult::Wait(_) => {} // expected
                CoalesceResult::Proceed(_) => panic!("second caller should wait"),
            }
        }

        #[test]
        fn test_different_urls_both_proceed() {
            let coalescer = RequestCoalescer::new();
            let _g1 = match coalescer.try_start("https://a.com") {
                CoalesceResult::Proceed(g) => g,
                CoalesceResult::Wait(_) => panic!("should proceed"),
            };
            let _g2 = match coalescer.try_start("https://b.com") {
                CoalesceResult::Proceed(g) => g,
                CoalesceResult::Wait(_) => panic!("should proceed for different URL"),
            };
            assert_eq!(coalescer.in_flight_count(), 2);
        }

        #[test]
        fn test_guard_drop_removes_entry() {
            let coalescer = RequestCoalescer::new();
            {
                let _guard = match coalescer.try_start("https://example.com") {
                    CoalesceResult::Proceed(g) => g,
                    CoalesceResult::Wait(_) => panic!("should proceed"),
                };
                assert_eq!(coalescer.in_flight_count(), 1);
                // guard dropped here
            }
            assert_eq!(coalescer.in_flight_count(), 0);

            // Next caller for same URL should proceed.
            match coalescer.try_start("https://example.com") {
                CoalesceResult::Proceed(_) => {} // expected
                CoalesceResult::Wait(_) => panic!("should proceed after guard dropped"),
            }
        }

        #[test]
        fn test_complete_removes_entry() {
            let coalescer = RequestCoalescer::new();
            let guard = match coalescer.try_start("https://example.com") {
                CoalesceResult::Proceed(g) => g,
                CoalesceResult::Wait(_) => panic!("should proceed"),
            };
            assert_eq!(coalescer.in_flight_count(), 1);
            guard.complete();
            assert_eq!(coalescer.in_flight_count(), 0);
        }

        #[tokio::test]
        async fn test_waiter_notified_on_complete() {
            let coalescer = Arc::new(RequestCoalescer::new());
            let guard = match coalescer.try_start("https://example.com") {
                CoalesceResult::Proceed(g) => g,
                CoalesceResult::Wait(_) => panic!("should proceed"),
            };

            let mut rx = match coalescer.try_start("https://example.com") {
                CoalesceResult::Wait(r) => r,
                CoalesceResult::Proceed(_) => panic!("should wait"),
            };

            // Complete in a separate task.
            let handle = tokio::spawn(async move {
                guard.complete();
            });

            // The receiver should get notified.
            let result = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;
            assert!(result.is_ok(), "should receive notification");
            handle.await.unwrap();
        }

        #[test]
        fn test_purge_stale_no_panic_on_empty() {
            let coalescer = RequestCoalescer::new();
            coalescer.purge_stale(); // Should not panic.
            assert_eq!(coalescer.in_flight_count(), 0);
        }

        #[test]
        fn test_double_complete_is_safe() {
            let coalescer = RequestCoalescer::new();
            let mut guard = match coalescer.try_start("https://example.com") {
                CoalesceResult::Proceed(g) => g,
                CoalesceResult::Wait(_) => panic!("should proceed"),
            };
            // Manually call finish twice (simulates complete + drop).
            guard.finish();
            guard.finish();
            assert_eq!(coalescer.in_flight_count(), 0);
        }
    }
}

#[cfg(feature = "request_coalesce")]
pub use inner::{CoalesceGuard, CoalesceResult, RequestCoalescer};
