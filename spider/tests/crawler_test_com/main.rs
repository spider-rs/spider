//! Comprehensive integration test suite based on <https://www.crawler-test.com/>.
//!
//! Gated behind `RUN_LIVE_TESTS` env var. Requires `basic` feature at minimum.
//! Smart-mode tests require `smart` feature. Chrome-mode tests require `chrome` feature.
//!
//! Run:
//!   RUN_LIVE_TESTS=1 cargo test --test crawler_test_com --features "basic"
//!   RUN_LIVE_TESTS=1 cargo test --test crawler_test_com --features "basic,smart"
//!   RUN_LIVE_TESTS=1 cargo test --test crawler_test_com --features "basic,chrome"

mod helpers;

mod status_codes;
mod redirects;
mod links;
mod urls;
mod canonical_tags;
mod robots_protocol;
mod content;
mod javascript;
mod encoding;
mod titles;
mod description_tags;
mod social_tags;
mod mobile;
mod other;
