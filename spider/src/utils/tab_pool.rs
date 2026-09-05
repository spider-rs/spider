/// Pool of reusable Chrome CDP tabs backed by a bounded lock-free queue.
/// Idle tabs are reused in FIFO order. Browser I/O happens outside the queue.
pub struct TabPool {
    slots: PoolSlots<chromiumoxide::Page>,
}

struct PoolSlots<T> {
    // ArrayQueue requires nonzero capacity; None represents disabled pooling.
    values: Option<crossbeam_queue::ArrayQueue<T>>,
    max_size: usize,
}

impl<T> PoolSlots<T> {
    fn new(max_size: usize) -> Self {
        Self {
            values: (max_size > 0).then(|| crossbeam_queue::ArrayQueue::new(max_size)),
            max_size,
        }
    }

    fn pop(&self) -> Option<T> {
        self.values.as_ref().and_then(|queue| queue.pop())
    }

    fn push(&self, value: T) -> Result<(), T> {
        match &self.values {
            Some(queue) => queue.push(value),
            None => Err(value),
        }
    }

    fn drain(&self) -> Vec<T> {
        // Bound the sweep so concurrent producers cannot keep clear running
        // indefinitely. Releases racing the sweep may remain for later reuse.
        (0..self.len()).filter_map(|_| self.pop()).collect()
    }

    fn len(&self) -> usize {
        self.values.as_ref().map_or(0, |queue| queue.len())
    }
}

/// Hand a pooled tab off to actually be closed.
///
/// Dropping a `chromiumoxide::Page` does **not** close the underlying CDP tab — it only
/// decrements an internal counter (see the note on
/// [`crate::features::chrome::TabCloseGuard`]). Every path in this pool that stops
/// holding a tab must route it through here, or the tab leaks and Chrome eventually
/// stops handing out new ones.
///
/// Synchronous and infallible so it is safe to call from non-async paths such as
/// [`TabPool::clear`], and from a `Drop` if this pool ever grows one.
#[inline]
fn close_pooled_tab(page: chromiumoxide::Page) {
    #[cfg(not(feature = "decentralized"))]
    {
        // Reuse the crate-wide background closer: it dedups by `target_id` and bounds
        // each close with its own timeout, so this is just a channel send.
        drop(crate::features::chrome::TabCloseGuard::new(page));
    }
    #[cfg(feature = "decentralized")]
    {
        // `TabCloseGuard` is compiled out under `decentralized`, so close inline on a
        // detached task with the same bound. `try_current` (never `current`) keeps this
        // a no-op instead of a panic when there is no runtime.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), page.close()).await;
            });
        }
    }
}

impl TabPool {
    /// Create a new tab pool with the given maximum size.
    pub fn new(max_size: usize) -> Self {
        Self {
            slots: PoolSlots::new(max_size),
        }
    }

    /// Acquire an idle tab, or create a new one.
    pub async fn acquire(
        &self,
        browser: &chromiumoxide::Browser,
    ) -> Result<chromiumoxide::Page, chromiumoxide::error::CdpError> {
        if let Some(page) = self.slots.pop() {
            return Ok(page);
        }
        browser.new_page("about:blank").await
    }

    /// Clear page state and return it to the pool, closing surplus tabs.
    pub async fn release(&self, page: chromiumoxide::Page) {
        if self.slots.len() >= self.slots.max_size {
            close_pooled_tab(page);
            return;
        }
        // Own cleanup across cancellation while navigation is pending.
        let guard = PendingTab(Some(page));
        let Some(page) = guard.0.as_ref() else {
            return;
        };
        let ok = matches!(
            tokio::time::timeout(std::time::Duration::from_secs(5), page.goto("about:blank")).await,
            Ok(Ok(_))
        );
        if !ok {
            return;
        }
        let mut guard = guard;
        if let Some(page) = guard.0.take() {
            if let Err(page) = self.slots.push(page) {
                close_pooled_tab(page);
            }
        }
    }

    /// Close all tabs idle at the moment of the drain. Concurrent releases
    /// after the drain remain in the pool for subsequent reuse or cleanup.
    pub fn clear(&self) {
        for page in self.slots.drain() {
            close_pooled_tab(page);
        }
    }

    /// Return the number of idle tabs.
    pub fn pool_size(&self) -> usize {
        self.slots.len()
    }
}

struct PendingTab(Option<chromiumoxide::Page>);
impl Drop for PendingTab {
    fn drop(&mut self) {
        if let Some(page) = self.0.take() {
            close_pooled_tab(page);
        }
    }
}

impl Drop for TabPool {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_pool_operations_never_lose_values() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        // Count explicit recoveries, not Drop: dropping a Page handle silently
        // would leak the remote tab even though its Rust value was destroyed.
        let recovered = Arc::new((0..8000).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        let pool = Arc::new(PoolSlots::new(8));
        let handles: Vec<_> = (0..8)
            .map(|worker| {
                let pool = pool.clone();
                let recovered = recovered.clone();
                std::thread::spawn(move || {
                    for i in 0..1000 {
                        if let Err(value) = pool.push(worker * 1000 + i) {
                            recovered[value].fetch_add(1, Ordering::Relaxed);
                        }
                        assert!(pool.len() <= 8);
                        if (worker + i) % 3 == 0 {
                            for value in pool.drain() {
                                recovered[value].fetch_add(1, Ordering::Relaxed);
                            }
                        } else if let Some(value) = pool.pop() {
                            recovered[value].fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        for value in pool.drain() {
            recovered[value].fetch_add(1, Ordering::Relaxed);
        }
        assert!(recovered
            .iter()
            .all(|count| count.load(Ordering::Relaxed) == 1));
    }

    #[test]
    fn pool_reuses_fifo_and_rejects_overflow() {
        let pool = PoolSlots::new(2);
        assert_eq!(pool.push(1), Ok(()));
        assert_eq!(pool.push(2), Ok(()));
        assert_eq!(pool.push(3), Err(3));
        assert_eq!(pool.pop(), Some(1));
        assert_eq!(pool.pop(), Some(2));
        assert_eq!(pool.pop(), None);
    }

    #[test]
    fn test_new_pool_is_empty() {
        let pool = TabPool::new(5);
        assert_eq!(pool.pool_size(), 0);
    }

    #[test]
    fn test_pool_max_size() {
        let pool = TabPool::new(0);
        assert_eq!(pool.slots.max_size, 0);

        let pool = TabPool::new(100);
        assert_eq!(pool.slots.max_size, 100);
    }

    #[test]
    fn test_clear_empty_pool() {
        let pool = TabPool::new(5);
        pool.clear();
        assert_eq!(pool.pool_size(), 0);
    }
}
