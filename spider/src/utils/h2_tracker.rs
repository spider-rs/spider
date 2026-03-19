#![cfg(feature = "h2_multiplex")]

use case_insensitive_string::compact_str::CompactString;
use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Default HTTP/2 max concurrent streams per origin (RFC 7540 default).
const DEFAULT_MAX_STREAMS: u32 = 100;

/// Per-origin HTTP/2 multiplexing state.
struct H2Origin {
    supports_h2: AtomicBool,
    active_streams: AtomicU32,
    max_streams: u32,
}

impl H2Origin {
    fn new(supports_h2: bool, max_streams: u32) -> Self {
        Self {
            supports_h2: AtomicBool::new(supports_h2),
            active_streams: AtomicU32::new(0),
            max_streams,
        }
    }
}

/// Tracks per-origin HTTP/2 state to avoid unnecessary semaphore permits.
///
/// When an origin is known to support HTTP/2, multiple requests can share a
/// single TCP connection via stream multiplexing. This tracker records which
/// origins support H2 and manages stream counts so the crawler can skip
/// per-request connection semaphore permits for multiplexed origins.
pub struct H2Tracker {
    origins: DashMap<CompactString, H2Origin>,
}

impl H2Tracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self {
            origins: DashMap::with_capacity(64),
        }
    }

    /// Record the HTTP protocol version observed for a given origin.
    ///
    /// Call this after receiving a response to teach the tracker whether
    /// the origin speaks HTTP/2.
    pub fn record_protocol(&self, origin: &str, version: http::Version) {
        let is_h2 = version == http::Version::HTTP_2;
        let key = CompactString::new(origin);

        match self.origins.get(&key) {
            Some(entry) => {
                entry.supports_h2.store(is_h2, Ordering::Release);
            }
            None => {
                self.origins
                    .insert(key, H2Origin::new(is_h2, DEFAULT_MAX_STREAMS));
            }
        }
    }

    /// Try to claim a multiplexed stream slot for the given origin.
    ///
    /// Returns `true` if the origin supports HTTP/2 **and** has spare stream
    /// capacity (active < max_streams). On success, increments the active
    /// stream count atomically. The caller **must** call [`release_stream`]
    /// when the request completes.
    ///
    /// Returns `false` if the origin does not support H2, is unknown, or is
    /// at capacity.
    pub fn try_multiplex(&self, origin: &str) -> bool {
        let key = CompactString::new(origin);

        if let Some(entry) = self.origins.get(&key) {
            if !entry.supports_h2.load(Ordering::Acquire) {
                return false;
            }

            // CAS loop to atomically increment if below max.
            loop {
                let current = entry.active_streams.load(Ordering::Acquire);
                if current >= entry.max_streams {
                    return false;
                }
                match entry.active_streams.compare_exchange_weak(
                    current,
                    current + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return true,
                    Err(_) => continue,
                }
            }
        } else {
            false
        }
    }

    /// Release a multiplexed stream slot for the given origin.
    ///
    /// Decrements the active stream count. Safe to call even if the origin
    /// is unknown (no-op in that case).
    pub fn release_stream(&self, origin: &str) {
        let key = CompactString::new(origin);

        if let Some(entry) = self.origins.get(&key) {
            // Saturating decrement: avoid underflow if called spuriously.
            entry
                .active_streams
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| {
                    if v > 0 {
                        Some(v - 1)
                    } else {
                        None
                    }
                })
                .ok();
        }
    }

    /// Check whether an origin is known to support HTTP/2.
    pub fn supports_h2(&self, origin: &str) -> bool {
        let key = CompactString::new(origin);

        self.origins
            .get(&key)
            .map(|e| e.supports_h2.load(Ordering::Acquire))
            .unwrap_or(false)
    }
}

impl Default for H2Tracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unknown_origin_not_h2() {
        let tracker = H2Tracker::new();
        assert!(!tracker.supports_h2("https://example.com"));
        assert!(!tracker.try_multiplex("https://example.com"));
    }

    #[test]
    fn test_record_h2_origin() {
        let tracker = H2Tracker::new();
        tracker.record_protocol("https://example.com", http::Version::HTTP_2);
        assert!(tracker.supports_h2("https://example.com"));

        // H1 origin should not be marked as H2.
        tracker.record_protocol("https://h1.example.com", http::Version::HTTP_11);
        assert!(!tracker.supports_h2("https://h1.example.com"));
    }

    #[test]
    fn test_multiplex_and_release() {
        let tracker = H2Tracker::new();
        tracker.record_protocol("https://example.com", http::Version::HTTP_2);

        // Should succeed — well within stream limit.
        assert!(tracker.try_multiplex("https://example.com"));
        assert!(tracker.try_multiplex("https://example.com"));

        // Release both.
        tracker.release_stream("https://example.com");
        tracker.release_stream("https://example.com");

        // Extra release should be a safe no-op (no underflow).
        tracker.release_stream("https://example.com");
        tracker.release_stream("https://unknown.com");
    }

    #[test]
    fn test_stream_capacity_limit() {
        let tracker = H2Tracker::new();
        tracker.record_protocol("https://example.com", http::Version::HTTP_2);

        // Exhaust all stream slots.
        for _ in 0..DEFAULT_MAX_STREAMS {
            assert!(tracker.try_multiplex("https://example.com"));
        }

        // Next attempt should fail — at capacity.
        assert!(!tracker.try_multiplex("https://example.com"));

        // Release one and try again.
        tracker.release_stream("https://example.com");
        assert!(tracker.try_multiplex("https://example.com"));
    }

    #[test]
    fn test_h1_origin_cannot_multiplex() {
        let tracker = H2Tracker::new();
        tracker.record_protocol("https://h1only.com", http::Version::HTTP_11);
        assert!(!tracker.try_multiplex("https://h1only.com"));
    }

    #[test]
    fn test_protocol_downgrade() {
        let tracker = H2Tracker::new();
        tracker.record_protocol("https://example.com", http::Version::HTTP_2);
        assert!(tracker.supports_h2("https://example.com"));

        // Server switches to H1 (e.g., fallback).
        tracker.record_protocol("https://example.com", http::Version::HTTP_11);
        assert!(!tracker.supports_h2("https://example.com"));
        assert!(!tracker.try_multiplex("https://example.com"));
    }
}
