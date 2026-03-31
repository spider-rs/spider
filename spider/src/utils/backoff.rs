use std::time::Duration;

/// Simple pseudo-random jitter using thread-local state.
/// Avoids external dependencies while still providing reasonable spread.
#[inline]
fn cheap_jitter(bound: u64) -> u64 {
    if bound == 0 {
        return 0;
    }
    // Use the current instant's low bits as entropy source.
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    // xorshift-style mixing.
    let mut x = seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x % bound
}

/// Compute exponential backoff with full jitter.
///
/// Formula: `min(base_ms * 2^attempt + jitter, max_ms)`
/// where `jitter` is a pseudo-random value in `[0, base_ms)`.
///
/// Uses checked arithmetic throughout — no panics on overflow.
#[inline]
pub fn backoff_delay(attempt: u32, base_ms: u64, max_ms: u64) -> Duration {
    let exp = base_ms.saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
    let jitter = cheap_jitter(base_ms.max(1));
    let delay = exp.saturating_add(jitter).min(max_ms);
    Duration::from_millis(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_zero_returns_near_base() {
        let d = backoff_delay(0, 100, 10_000);
        // attempt=0 → 100 * 1 + jitter(0..100) → [100, 199]
        assert!(d.as_millis() >= 100);
        assert!(d.as_millis() < 200);
    }

    #[test]
    fn delay_grows_exponentially() {
        // attempt=3 base delay = 100*8 = 800, attempt=0 base = 100
        // Even with jitter, attempt 3 minimum (800) > attempt 0 maximum (199).
        let high = backoff_delay(3, 100, 100_000);
        assert!(
            high.as_millis() >= 800,
            "attempt 3 should be at least 800ms, got {}",
            high.as_millis()
        );
    }

    #[test]
    fn respects_max_ms_cap() {
        let d = backoff_delay(30, 1000, 5000);
        assert!(d.as_millis() <= 5000);
    }

    #[test]
    fn jitter_is_bounded() {
        for _ in 0..100 {
            let d = backoff_delay(0, 100, 100_000);
            assert!(d.as_millis() >= 100);
            assert!(d.as_millis() < 200);
        }
    }

    #[test]
    fn huge_attempt_saturates_without_panic() {
        let d = backoff_delay(u32::MAX, 1000, 60_000);
        assert!(d.as_millis() <= 60_000);
    }

    #[test]
    fn zero_base_ms_does_not_panic() {
        let d = backoff_delay(5, 0, 10_000);
        assert!(d.as_millis() <= 1);
    }

    #[test]
    fn zero_max_ms_returns_zero() {
        let d = backoff_delay(3, 100, 0);
        assert_eq!(d.as_millis(), 0);
    }

    /// Verify backoff_delay is safe under concurrent access — no panics,
    /// no deadlocks, all values within expected bounds.
    #[test]
    fn concurrent_safety_no_panic_or_deadlock() {
        use std::sync::{Arc, Barrier};

        let threads = 32;
        let iterations = 500;
        let barrier = Arc::new(Barrier::new(threads));

        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait(); // all threads start simultaneously
                    for i in 0..iterations {
                        let attempt = ((t * iterations + i) % 20) as u32;
                        let d = backoff_delay(attempt, 200, 15_000);
                        assert!(
                            d.as_millis() <= 15_000,
                            "thread {t} iter {i}: delay {}ms exceeds cap",
                            d.as_millis()
                        );
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("thread must not panic");
        }
    }

    /// Verify monotonic growth tendency across attempts (ignoring jitter).
    /// With base=200, attempt N base delay = 200*2^N. Even with jitter in
    /// [0, 200), attempt 4 minimum (3200) > attempt 0 maximum (399).
    #[test]
    fn monotonic_growth_across_attempts() {
        let low = backoff_delay(0, 200, 100_000);
        let high = backoff_delay(4, 200, 100_000);
        assert!(
            high >= low,
            "attempt 4 ({:?}) should be >= attempt 0 ({:?})",
            high,
            low
        );
        // Stronger: attempt 4 base = 200*16 = 3200, min is 3200 vs max of attempt 0 = 399
        assert!(high.as_millis() >= 3200);
    }

    /// Verify base_ms=1 doesn't underflow or produce zero jitter panics.
    #[test]
    fn base_ms_one_no_underflow() {
        for attempt in 0..20 {
            let d = backoff_delay(attempt, 1, 60_000);
            assert!(d.as_millis() >= 1u128.min(1u128 << attempt as u128));
            assert!(d.as_millis() <= 60_000);
        }
    }
}
