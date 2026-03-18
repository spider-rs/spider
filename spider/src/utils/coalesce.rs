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
        completed: bool,
    }

    struct InFlightEntry {
        sender: broadcast::Sender<()>,
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

            if let Some((_, entry)) = self.in_flight.remove(&self.url) {
                // Notify all waiters. Ignore send errors (no active receivers is fine).
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
    /// Thread-safe, lock-free at the shard level. The `DashMap` shard lock is
    /// never held across an `.await` point, so deadlocks are impossible.
    pub struct RequestCoalescer {
        in_flight: Arc<DashMap<CompactString, InFlightEntry>>,
    }

    impl RequestCoalescer {
        /// Create a new coalescer.
        pub fn new() -> Self {
            Self {
                in_flight: Arc::new(DashMap::new()),
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

            // Check for an existing in-flight entry first (read path, cheaper).
            if let Some(entry) = self.in_flight.get(&key) {
                // Check for stale entries — if the guard has been held too long,
                // treat it as abandoned and let this caller take over.
                if entry.created_at.elapsed() < Duration::from_secs(STALE_TIMEOUT_SECS) {
                    let rx = entry.sender.subscribe();
                    return CoalesceResult::Wait(rx);
                }
                // Entry is stale — fall through to replace it.
                drop(entry); // Release the DashMap shard lock before mutating.
                self.in_flight.remove(&key);
            }

            // No in-flight entry (or it was stale). Insert a new one.
            let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
            let entry = InFlightEntry {
                sender: tx,
                created_at: Instant::now(),
            };
            self.in_flight.insert(key.clone(), entry);

            CoalesceResult::Proceed(CoalesceGuard {
                url: key,
                in_flight: Arc::clone(&self.in_flight),
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
