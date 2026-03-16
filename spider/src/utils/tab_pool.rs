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
    /// If the navigation hangs for more than 5 seconds the tab is dropped.
    /// If the pool is already at capacity the tab is also dropped (closed).
    pub async fn release(&self, page: chromiumoxide::Page) {
        let current = self.head.load(std::sync::atomic::Ordering::Relaxed);
        if current >= self.max_size {
            return; // at capacity, drop the page
        }

        // Navigate to about:blank with a 5s timeout to clear state.
        let ok = matches!(
            tokio::time::timeout(std::time::Duration::from_secs(5), page.goto("about:blank")).await,
            Ok(Ok(_))
        );

        if !ok {
            return; // navigation failed/timed out, drop the page
        }

        // Try to push onto the stack.
        loop {
            let current = self.head.load(std::sync::atomic::Ordering::Acquire);
            if current >= self.max_size {
                return; // pool filled while we were navigating
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

    /// Drop all pooled tabs by draining the map.
    pub fn clear(&self) {
        self.head.store(0, std::sync::atomic::Ordering::Release);
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
