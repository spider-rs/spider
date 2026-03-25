//! Integration tests for the `parallel_backends` feature.
//!
//! These tests verify configuration, builder logic, and graceful handling
//! without requiring real LightPanda / Servo instances. Tests that need
//! live endpoints are gated behind env vars `LIGHTPANDA_CDP_URL` and
//! `SERVO_WEBDRIVER_URL`.
#![cfg(feature = "parallel_backends")]

use spider::configuration::{BackendEndpoint, BackendEngine, ParallelBackendsConfig};
use spider::utils::parallel_backends::{
    html_quality_score, BackendResponse, BackendResult, BackendTracker, ProxyRotator,
};

// ---------------------------------------------------------------------------
// Config Tests
// ---------------------------------------------------------------------------

#[test]
fn test_config_default_values() {
    let cfg = ParallelBackendsConfig::default();
    assert_eq!(cfg.grace_period_ms, 500);
    assert_eq!(cfg.fast_accept_threshold, 80);
    assert_eq!(cfg.max_consecutive_errors, 10);
    assert!(cfg.enabled);
    assert!(cfg.backends.is_empty());
}

#[cfg(feature = "serde")]
#[test]
fn test_config_serde_roundtrip() {
    let cfg = ParallelBackendsConfig {
        backends: vec![
            BackendEndpoint {
                engine: BackendEngine::LightPanda,
                endpoint: Some("ws://127.0.0.1:9222".to_string()),
                binary_path: None,
                protocol: None,
            },
            BackendEndpoint {
                engine: BackendEngine::Servo,
                endpoint: Some("http://localhost:4444".to_string()),
                binary_path: None,
                protocol: None,
            },
        ],
        grace_period_ms: 250,
        enabled: true,
        fast_accept_threshold: 90,
        max_consecutive_errors: 5,
    };

    let json = serde_json::to_string(&cfg).unwrap();
    let deserialized: ParallelBackendsConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(cfg, deserialized);
}

// ---------------------------------------------------------------------------
// Builder Tests
// ---------------------------------------------------------------------------

#[test]
fn test_build_backend_futures_empty_config() {
    let cfg = ParallelBackendsConfig::default();
    let crawl_cfg = std::sync::Arc::new(spider::configuration::Configuration::default());
    let tracker = BackendTracker::new(1, 10);
    let futs = spider::utils::parallel_backends::build_backend_futures(
        "https://example.com",
        &cfg,
        &crawl_cfg,
        &tracker,
        &None,
    );
    assert!(futs.is_empty());
}

#[test]
fn test_build_backend_futures_skips_disabled() {
    let cfg = ParallelBackendsConfig {
        backends: vec![BackendEndpoint {
            engine: BackendEngine::LightPanda,
            endpoint: Some("ws://localhost:9222".to_string()),
            binary_path: None,
            protocol: None,
        }],
        ..Default::default()
    };
    let crawl_cfg = std::sync::Arc::new(spider::configuration::Configuration::default());
    let tracker = BackendTracker::new(2, 1);
    // Trigger auto-disable by recording max_consecutive_errors errors.
    tracker.record_error(1);
    assert!(tracker.is_disabled(1));

    let futs = spider::utils::parallel_backends::build_backend_futures(
        "https://example.com",
        &cfg,
        &crawl_cfg,
        &tracker,
        &None,
    );
    // Backend 1 is disabled → no futures built.
    assert!(futs.is_empty());
}

#[test]
fn test_build_backend_futures_skips_local_stub() {
    let cfg = ParallelBackendsConfig {
        backends: vec![BackendEndpoint {
            engine: BackendEngine::Servo,
            endpoint: None,
            binary_path: Some("/usr/bin/servo".to_string()),
            protocol: None,
        }],
        ..Default::default()
    };
    let crawl_cfg = std::sync::Arc::new(spider::configuration::Configuration::default());
    let tracker = BackendTracker::new(2, 10);

    let futs = spider::utils::parallel_backends::build_backend_futures(
        "https://example.com",
        &cfg,
        &crawl_cfg,
        &tracker,
        &None,
    );
    // Local mode is a stub → no futures.
    assert!(futs.is_empty());
}

// ---------------------------------------------------------------------------
// Quality Scorer Integration
// ---------------------------------------------------------------------------

#[test]
fn test_quality_score_real_html() {
    let html = br#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>Example</title></head>
<body>
<h1>Welcome to Example.com</h1>
<p>This is a real page with meaningful content that should score highly.</p>
<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod
tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam,
quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.</p>
<a href="/about">About</a>
<a href="/contact">Contact</a>
</body>
</html>"#;

    let score = html_quality_score(
        Some(html),
        reqwest::StatusCode::OK,
        &spider::page::AntiBotTech::None,
    );
    // Should be high: 200 + content + body tag + not empty + no bot = ~100
    assert!(score >= 90, "Expected >= 90, got {}", score);
}

#[test]
fn test_quality_score_antibot_page() {
    let html = b"<html><body>Access denied</body></html>";
    let score = html_quality_score(
        Some(html),
        reqwest::StatusCode::FORBIDDEN,
        &spider::page::AntiBotTech::Cloudflare,
    );
    // Anti-bot + 403 = very low
    assert!(score < 50, "Expected < 50, got {}", score);
}

// ---------------------------------------------------------------------------
// Tracker Stress Test
// ---------------------------------------------------------------------------

#[test]
fn test_tracker_concurrent_stress() {
    use std::sync::Arc;

    let tracker = Arc::new(BackendTracker::new(4, 100));
    let handles: Vec<_> = (0..8)
        .map(|thread_id| {
            let t = tracker.clone();
            std::thread::spawn(move || {
                for i in 0..1000 {
                    let idx = (thread_id + i) % 4;
                    t.record_race(idx);
                    t.record_win(idx);
                    t.record_duration(idx, std::time::Duration::from_millis(100));
                    t.record_success(idx);
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // Total races should be 8 * 1000 = 8000, distributed across 4 backends.
    let total_races: u64 = (0..4).map(|i| tracker.races(i)).sum();
    assert_eq!(total_races, 8000);
    let total_wins: u64 = (0..4).map(|i| tracker.wins(i)).sum();
    assert_eq!(total_wins, 8000);
    // No backend should be disabled.
    for i in 0..4 {
        assert!(!tracker.is_disabled(i));
    }
}

// ---------------------------------------------------------------------------
// Proxy Rotator Integration
// ---------------------------------------------------------------------------

#[test]
fn test_proxy_rotator_from_config() {
    use spider::configuration::{ProxyIgnore, RequestProxy};

    let proxies = vec![
        RequestProxy {
            addr: "socks5://proxy1:1080".into(),
            ignore: ProxyIgnore::No,
        },
        RequestProxy {
            addr: "http://proxy2:8080".into(),
            ignore: ProxyIgnore::Chrome,
        },
        RequestProxy {
            addr: "http://proxy3:8080".into(),
            ignore: ProxyIgnore::Http,
        },
    ];

    let rotator = ProxyRotator::new(&Some(proxies));

    // CDP: proxy1 (No) + proxy3 (Http→not Chrome→included)
    assert_eq!(rotator.cdp_count(), 2);
    // WebDriver: proxy1 (No) + proxy2 (Chrome→not Http→included)
    assert_eq!(rotator.webdriver_count(), 2);

    // Round-robin works.
    let first = rotator.next_cdp().unwrap();
    let second = rotator.next_cdp().unwrap();
    assert_ne!(first, second);
}

// ---------------------------------------------------------------------------
// Race Orchestrator with real tokio runtime
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_race_fast_accept_under_load() {
    use std::pin::Pin;

    let tracker = BackendTracker::new(3, 10);
    let cfg = ParallelBackendsConfig {
        backends: vec![],
        grace_period_ms: 1000,
        enabled: true,
        fast_accept_threshold: 80,
        max_consecutive_errors: 10,
    };

    // Primary scores above threshold — should return immediately.
    let primary: Pin<Box<dyn std::future::Future<Output = Option<BackendResponse>> + Send>> =
        Box::pin(async {
            Some(BackendResponse {
                page: spider::page::Page::default(),
                quality_score: 95,
                backend_index: 0,
                duration: std::time::Duration::from_millis(10),
            })
        });

    // Slow alternatives that would win if given time.
    let alt1: Pin<Box<dyn std::future::Future<Output = BackendResult> + Send>> = Box::pin(async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        BackendResult {
            backend_index: 1,
            response: Some(BackendResponse {
                page: spider::page::Page::default(),
                quality_score: 100,
                backend_index: 1,
                duration: std::time::Duration::from_secs(5),
            }),
        }
    });

    let result =
        spider::utils::parallel_backends::race_backends(primary, vec![alt1], &cfg, &tracker).await;
    let r = result.unwrap();

    // Primary should win via fast-accept despite lower score than alt.
    assert_eq!(r.backend_index, 0);
    assert_eq!(r.quality_score, 95);
}

#[tokio::test]
async fn test_race_disabled_returns_primary_directly() {
    let tracker = BackendTracker::new(2, 10);
    let mut cfg = ParallelBackendsConfig::default();
    cfg.enabled = false;

    let primary: std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<BackendResponse>> + Send>,
    > = Box::pin(async {
        Some(BackendResponse {
            page: spider::page::Page::default(),
            quality_score: 50,
            backend_index: 0,
            duration: std::time::Duration::from_millis(10),
        })
    });

    let alt: std::pin::Pin<Box<dyn std::future::Future<Output = BackendResult> + Send>> =
        Box::pin(async {
            BackendResult {
                backend_index: 1,
                response: Some(BackendResponse {
                    page: spider::page::Page::default(),
                    quality_score: 100,
                    backend_index: 1,
                    duration: std::time::Duration::from_millis(1),
                }),
            }
        });

    let result =
        spider::utils::parallel_backends::race_backends(primary, vec![alt], &cfg, &tracker).await;
    let r = result.unwrap();
    assert_eq!(r.backend_index, 0); // disabled → primary only
}
