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

mod canonical_tags;
mod content;
mod description_tags;
mod encoding;
mod javascript;
mod links;
mod mobile;
mod other;
mod redirects;
mod robots_protocol;
mod social_tags;
mod status_codes;
mod titles;
mod urls;
