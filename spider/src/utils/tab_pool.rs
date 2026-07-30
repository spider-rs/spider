/// Pool of reusable Chrome CDP tabs.
///
/// Lock-free design: uses a DashMap as a concurrent stack (push/pop by
/// atomic index). No Mutex, no RwLock.
pub struct TabPool {
    /// Tabs stored by slot index. DashMap provides lock-free per-shard access.
    slots: dashmap::DashMap<usize, chromiumoxide::Page>,
    /// Next slot to write into (monotonically increasing).
    head: std::sync::atomic::AtomicUsize,
    /// Maximum pool capacity.
    max_size: usize,
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
            slots: dashmap::DashMap::with_capacity(max_size),
            head: std::sync::atomic::AtomicUsize::new(0),
            max_size,
        }
    }

    /// Acquire a tab from the pool or create a new one.
    ///
    /// Pops the most recently pooled tab (LIFO) if available, otherwise
    /// creates a fresh tab via `browser.new_page("about:blank")`.
    pub async fn acquire(
        &self,
        browser: &chromiumoxide::Browser,
    ) -> Result<chromiumoxide::Page, chromiumoxide::error::CdpError> {
        // Try to pop from the stack (LIFO).
        loop {
            let current = self.head.load(std::sync::atomic::Ordering::Acquire);
            if current == 0 {
                break; // pool empty
            }
            let target = current - 1;
            // CAS to claim this slot.
            if self
                .head
                .compare_exchange(
                    current,
                    target,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .is_ok()
            {
                // We won the slot — remove and return the tab.
                if let Some((_, page)) = self.slots.remove(&target) {
                    return Ok(page);
                }
                // Slot was empty (shouldn't happen), continue to create new.
                break;
            }
            // CAS failed — another thread popped; retry.
        }
        browser.new_page("about:blank").await
    }

    /// Release a tab back to the pool.
    ///
    /// Navigates the tab to `about:blank` to clear state before pooling.
    /// If the navigation hangs for more than 5 seconds the tab is closed.
    /// If the pool is already at capacity the tab is closed instead of pooled.
    pub async fn release(&self, page: chromiumoxide::Page) {
        let current = self.head.load(std::sync::atomic::Ordering::Relaxed);
        if current >= self.max_size {
            close_pooled_tab(page); // at capacity — close, don't just drop
            return;
        }

        // Navigate to about:blank with a 5s timeout to clear state.
        let ok = matches!(
            tokio::time::timeout(std::time::Duration::from_secs(5), page.goto("about:blank")).await,
            Ok(Ok(_))
        );

        if !ok {
            close_pooled_tab(page); // navigation failed/timed out — close it
            return;
        }

        // Try to push onto the stack.
        loop {
            let current = self.head.load(std::sync::atomic::Ordering::Acquire);
            if current >= self.max_size {
                close_pooled_tab(page); // pool filled while we were navigating
                return;
            }
            if self
                .head
                .compare_exchange(
                    current,
                    current + 1,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .is_ok()
            {
                self.slots.insert(current, page);
                return;
            }
            // CAS failed — another thread pushed; retry.
        }
    }

    /// Close all pooled tabs and empty the pool.
    ///
    /// Takes ownership of each tab so it can be routed to the closer;
    /// `DashMap::clear` alone would only drop the `Page` handles, which leaves the
    /// CDP tabs open in the browser.
    pub fn clear(&self) {
        self.head.store(0, std::sync::atomic::Ordering::Release);

        // Collect keys first so the iteration (and its shard guards) finishes before
        // any `remove` — iterating a `DashMap` while mutating it can deadlock.
        let keys: Vec<usize> = self.slots.iter().map(|entry| *entry.key()).collect();

        for key in keys {
            if let Some((_, page)) = self.slots.remove(&key) {
                close_pooled_tab(page);
            }
        }

        // Sweep anything a concurrent `release` inserted while we were draining.
        // These handles are dropped rather than closed, which is the pre-existing
        // behavior for a tab that races a `clear`; the window is a single insert wide.
        self.slots.clear();
    }

    /// Returns the approximate number of pooled (idle) tabs.
    pub fn pool_size(&self) -> usize {
        self.head.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_pool_is_empty() {
        let pool = TabPool::new(5);
        assert_eq!(pool.pool_size(), 0);
    }

    #[test]
    fn test_pool_max_size() {
        let pool = TabPool::new(0);
        assert_eq!(pool.max_size, 0);

        let pool = TabPool::new(100);
        assert_eq!(pool.max_size, 100);
    }

    #[test]
    fn test_clear_empty_pool() {
        let pool = TabPool::new(5);
        pool.clear();
        assert_eq!(pool.pool_size(), 0);
    }
}
