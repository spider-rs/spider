use std::time::Duration;

/// Thread-local xorshift64 PRNG — fast, no external deps, no locks.
/// Each thread gets independent state seeded from system time + thread ID,
/// so concurrent crawlers produce different sequences.
#[inline]
fn thread_rng_u64(bound: u64) -> u64 {
    use std::cell::Cell;

    thread_local! {
        static STATE: Cell<u64> = Cell::new({
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos() as u64;
            // Use stack address as thread-unique entropy (different per thread).
            let stack_addr = &nanos as *const u64 as u64;
            nanos ^ stack_addr.wrapping_mul(0x9E3779B97F4A7C15)
        });
    }

    if bound == 0 {
        return 0;
    }

    STATE.with(|cell| {
        let mut x = cell.get();
        // xorshift64
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        cell.set(x);
        x % bound
    })
}

/// Compute exponential backoff with full jitter.
///
/// The delay is a uniform random value in `[0, cap]` where
/// `cap = min(base_ms * 2^attempt, max_ms)`.
///
/// This gives maximum spread between concurrent retriers at every
/// tick level — no two crawlers will cluster on the same delay.
///
/// Uses checked arithmetic throughout — no panics on overflow.
#[inline]
pub fn backoff_delay(attempt: u32, base_ms: u64, max_ms: u64) -> Duration {
    let exp = base_ms.saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
    let cap = exp.min(max_ms);
    // Full jitter: uniform in [0, cap]
    let delay = thread_rng_u64(cap.saturating_add(1));
    Duration::from_millis(delay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_zero_within_cap() {
        // Full jitter: attempt=0, base=100 → cap=100, delay in [0, 100]
        for _ in 0..100 {
            let d = backoff_delay(0, 100, 10_000);
            assert!(d.as_millis() <= 100);
        }
    }

    #[test]
    fn higher_attempt_has_higher_cap() {
        // attempt=3, base=100 → cap=800, delay in [0, 800]
        // attempt=0 → cap=100, delay in [0, 100]
        // Over many samples, attempt=3 average (~400) >> attempt=0 average (~50)
        let mut sum_low: u128 = 0;
        let mut sum_high: u128 = 0;
        let n = 200;
        for _ in 0..n {
            sum_low += backoff_delay(0, 100, 100_000).as_millis();
            sum_high += backoff_delay(3, 100, 100_000).as_millis();
        }
        assert!(
            sum_high > sum_low,
            "attempt 3 average should exceed attempt 0 average"
        );
    }

    #[test]
    fn respects_max_ms_cap() {
        for _ in 0..100 {
            let d = backoff_delay(30, 1000, 5000);
            assert!(d.as_millis() <= 5000);
        }
    }

    #[test]
    fn full_jitter_produces_spread() {
        // Full jitter should produce different values across calls.
        // With cap=10000, getting 100 identical values is astronomically unlikely.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            seen.insert(backoff_delay(3, 1000, 100_000).as_millis());
        }
        assert!(
            seen.len() > 10,
            "full jitter should produce spread, got only {} distinct values",
            seen.len()
        );
    }

    #[test]
    fn huge_attempt_saturates_without_panic() {
        let d = backoff_delay(u32::MAX, 1000, 60_000);
        assert!(d.as_millis() <= 60_000);
    }

    #[test]
    fn zero_base_ms_does_not_panic() {
        let d = backoff_delay(5, 0, 10_000);
        assert_eq!(d.as_millis(), 0);
    }

    #[test]
    fn zero_max_ms_returns_zero() {
        let d = backoff_delay(3, 100, 0);
        assert_eq!(d.as_millis(), 0);
    }

    /// Verify backoff_delay is safe under concurrent access — no panics,
    /// no deadlocks, all values within expected bounds.
    /// Each thread gets independent PRNG state, so no contention.
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
                    barrier.wait();
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

    /// Concurrent threads should produce divergent sequences (thread-local seed
    /// includes stack address), preventing thundering herd.
    #[test]
    fn concurrent_threads_diverge() {
        use std::sync::{Arc, Barrier};

        let threads = 8;
        let barrier = Arc::new(Barrier::new(threads));

        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let seq: Vec<u128> = (0..20)
                        .map(|i| backoff_delay(i, 1000, 60_000).as_millis())
                        .collect();
                    seq
                })
            })
            .collect();

        let seqs: Vec<Vec<u128>> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // Count how many pairs of threads have identical sequences
        let mut identical_pairs = 0;
        for i in 0..seqs.len() {
            for j in (i + 1)..seqs.len() {
                if seqs[i] == seqs[j] {
                    identical_pairs += 1;
                }
            }
        }
        assert_eq!(
            identical_pairs, 0,
            "threads should produce different delay sequences"
        );
    }

    #[test]
    fn base_ms_one_stays_bounded() {
        for attempt in 0..20 {
            let d = backoff_delay(attempt, 1, 60_000);
            let cap = 1u64.checked_shl(attempt).unwrap_or(u64::MAX).min(60_000);
            assert!(d.as_millis() <= cap as u128);
        }
    }
}
