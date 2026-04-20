/// Lock-free crawl vitals for intelligent scaling decisions.
///
/// All counters are atomic with `Relaxed` ordering — suitable for
/// best-effort telemetry, not transactional consistency. The goal is
/// zero-overhead observation that lets the spool, concurrency limiter,
/// and backpressure systems make smart choices at scale.
///
/// Every operation is `#[inline]` and branchless on the hot path.
use std::sync::atomic::{AtomicU64, Ordering};

// ── Global counters ──────────────────────────────────────────────────────

/// HTTP/Chrome requests currently in-flight (spawned but not yet completed).
static REQUESTS_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);

/// Total requests completed since process start (success + error).
static REQUESTS_COMPLETED: AtomicU64 = AtomicU64::new(0);

/// Total request errors since process start (timeout, 5xx, connection refused, etc.).
static REQUEST_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Bytes currently being transferred over the network (response bodies
/// being streamed but not yet fully received).
static NETWORK_BYTES_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);

/// Total response bytes received since process start.
static NETWORK_BYTES_RECEIVED: AtomicU64 = AtomicU64::new(0);

/// Pages currently held in memory (not yet consumed or spooled).
static PAGES_IN_MEMORY: AtomicU64 = AtomicU64::new(0);

/// Chrome tabs currently in use (navigating or extracting HTML).
#[cfg(feature = "chrome")]
static CHROME_TABS_ACTIVE: AtomicU64 = AtomicU64::new(0);

// ── Request lifecycle ────────────────────────────────────────────────────

/// Call when a request is dispatched (HTTP send or Chrome navigate).
#[inline]
pub fn request_start() {
    REQUESTS_IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
}

/// Call when a request completes (success or error).
#[inline]
pub fn request_end() {
    REQUESTS_IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
    REQUESTS_COMPLETED.fetch_add(1, Ordering::Relaxed);
}

/// Call when a request ends in error.
#[inline]
pub fn request_error() {
    REQUEST_ERRORS.fetch_add(1, Ordering::Relaxed);
}

// ── Network bytes ────────────────────────────────────────────────────────

/// Add `n` bytes to the in-flight network counter (chunk received but
/// response not yet complete).
#[inline]
pub fn network_bytes_add(n: u64) {
    NETWORK_BYTES_IN_FLIGHT.fetch_add(n, Ordering::Relaxed);
}

/// Subtract `n` bytes from the in-flight network counter (response
/// complete or stream closed).
#[inline]
pub fn network_bytes_done(n: u64) {
    NETWORK_BYTES_IN_FLIGHT.fetch_sub(n, Ordering::Relaxed);
    NETWORK_BYTES_RECEIVED.fetch_add(n, Ordering::Relaxed);
}

// ── Pages in memory ──────────────────────────────────────────────────────

/// Increment pages held in memory.
#[inline]
pub fn page_add() {
    PAGES_IN_MEMORY.fetch_add(1, Ordering::Relaxed);
}

/// Decrement pages held in memory (saturating — never underflows).
#[inline]
pub fn page_drop() {
    let _ = PAGES_IN_MEMORY.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
        if v > 0 {
            Some(v - 1)
        } else {
            None
        }
    });
}

// ── Chrome tabs ──────────────────────────────────────────────────────────

/// Increment active Chrome tabs.
#[cfg(feature = "chrome")]
#[inline]
pub fn chrome_tab_open() {
    CHROME_TABS_ACTIVE.fetch_add(1, Ordering::Relaxed);
}

/// Decrement active Chrome tabs (saturating — never underflows).
#[cfg(feature = "chrome")]
#[inline]
pub fn chrome_tab_close() {
    let _ = CHROME_TABS_ACTIVE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
        if v > 0 {
            Some(v - 1)
        } else {
            None
        }
    });
}

// ── Snapshot (read-only) ─────────────────────────────────────────────────

/// A point-in-time snapshot of all vitals. Cheap to construct (7 atomic
/// loads with Relaxed ordering).
#[derive(Debug, Clone, Copy, Default)]
pub struct VitalsSnapshot {
    /// HTTP/Chrome requests currently in-flight.
    pub requests_in_flight: u64,
    /// Total requests completed since process start.
    pub requests_completed: u64,
    /// Total request errors since process start.
    pub request_errors: u64,
    /// Response bytes currently being streamed.
    pub network_bytes_in_flight: u64,
    /// Total response bytes received since process start.
    pub network_bytes_received: u64,
    /// Pages currently held in memory.
    pub pages_in_memory: u64,
    /// Chrome tabs currently navigating or extracting.
    pub chrome_tabs_active: u64,
}

/// Take a consistent-ish snapshot of all vitals. Each field is a single
/// atomic load — the snapshot is not transactionally consistent but is
/// good enough for scaling heuristics.
#[inline]
pub fn snapshot() -> VitalsSnapshot {
    VitalsSnapshot {
        requests_in_flight: REQUESTS_IN_FLIGHT.load(Ordering::Relaxed),
        requests_completed: REQUESTS_COMPLETED.load(Ordering::Relaxed),
        request_errors: REQUEST_ERRORS.load(Ordering::Relaxed),
        network_bytes_in_flight: NETWORK_BYTES_IN_FLIGHT.load(Ordering::Relaxed),
        network_bytes_received: NETWORK_BYTES_RECEIVED.load(Ordering::Relaxed),
        pages_in_memory: PAGES_IN_MEMORY.load(Ordering::Relaxed),
        #[cfg(feature = "chrome")]
        chrome_tabs_active: CHROME_TABS_ACTIVE.load(Ordering::Relaxed),
        #[cfg(not(feature = "chrome"))]
        chrome_tabs_active: 0,
    }
}

/// Get the error rate as a percentage (0–100). Returns 0 if no requests completed.
#[inline]
pub fn error_rate_pct() -> u64 {
    let completed = REQUESTS_COMPLETED.load(Ordering::Relaxed);
    if completed == 0 {
        return 0;
    }
    REQUEST_ERRORS.load(Ordering::Relaxed) * 100 / completed
}

/// Quick check: is the system under load? Returns true when in-flight
/// requests exceed the given threshold. Useful for fast-path decisions
/// without taking a full snapshot.
#[inline]
pub fn is_under_load(threshold: u64) -> bool {
    REQUESTS_IN_FLIGHT.load(Ordering::Relaxed) > threshold
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_lifecycle() {
        let before = REQUESTS_COMPLETED.load(Ordering::Relaxed);
        let before_flight = REQUESTS_IN_FLIGHT.load(Ordering::Relaxed);

        request_start();
        assert_eq!(
            REQUESTS_IN_FLIGHT.load(Ordering::Relaxed),
            before_flight + 1
        );

        request_end();
        assert_eq!(REQUESTS_IN_FLIGHT.load(Ordering::Relaxed), before_flight);
        assert_eq!(REQUESTS_COMPLETED.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn test_network_bytes() {
        let before = NETWORK_BYTES_IN_FLIGHT.load(Ordering::Relaxed);
        let before_total = NETWORK_BYTES_RECEIVED.load(Ordering::Relaxed);

        network_bytes_add(1000);
        assert_eq!(
            NETWORK_BYTES_IN_FLIGHT.load(Ordering::Relaxed),
            before + 1000
        );

        network_bytes_done(1000);
        assert_eq!(NETWORK_BYTES_IN_FLIGHT.load(Ordering::Relaxed), before);
        assert_eq!(
            NETWORK_BYTES_RECEIVED.load(Ordering::Relaxed),
            before_total + 1000
        );
    }

    #[test]
    fn test_page_tracking() {
        let before = PAGES_IN_MEMORY.load(Ordering::Relaxed);

        page_add();
        page_add();
        assert_eq!(PAGES_IN_MEMORY.load(Ordering::Relaxed), before + 2);

        page_drop();
        assert_eq!(PAGES_IN_MEMORY.load(Ordering::Relaxed), before + 1);
        page_drop();
        assert_eq!(PAGES_IN_MEMORY.load(Ordering::Relaxed), before);

        // Saturating — should not underflow.
        let current = PAGES_IN_MEMORY.load(Ordering::Relaxed);
        if current == 0 {
            page_drop();
            assert_eq!(PAGES_IN_MEMORY.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn test_snapshot() {
        let _snap = snapshot();
    }

    #[test]
    fn test_error_rate() {
        let _base_completed = REQUESTS_COMPLETED.load(Ordering::Relaxed);
        let _base_errors = REQUEST_ERRORS.load(Ordering::Relaxed);

        // Add 10 completions and 2 errors.
        for _ in 0..10 {
            request_start();
            request_end();
        }
        request_error();
        request_error();

        let rate = error_rate_pct();
        // Rate = (base_errors + 2) * 100 / (base_completed + 10)
        // Can't assert exact value due to parallel tests, just check it's bounded.
        assert!(rate <= 100);
    }

    #[cfg(feature = "chrome")]
    #[test]
    fn test_chrome_tabs() {
        let before = CHROME_TABS_ACTIVE.load(Ordering::Relaxed);

        chrome_tab_open();
        assert_eq!(CHROME_TABS_ACTIVE.load(Ordering::Relaxed), before + 1);

        chrome_tab_close();
        assert_eq!(CHROME_TABS_ACTIVE.load(Ordering::Relaxed), before);
    }
}
