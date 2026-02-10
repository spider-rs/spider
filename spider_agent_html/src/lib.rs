//! # Spider Agent HTML
//!
//! HTML processing utilities for spider_agent — cleaning, content analysis integration, and diffing.
//!
//! This crate provides the HTML cleaning functions extracted from `spider_agent`.
//! Uses `lol_html` for fast, streaming HTML rewriting.
//!
//! ## Dependencies
//!
//! - `lol_html` — streaming HTML rewriter
//! - `aho-corasick` — pattern matching (via spider_agent_types)
//! - `spider_agent_types` — type definitions

mod cleaning;

pub use cleaning::{
    clean_html, clean_html_base, clean_html_full, clean_html_raw, clean_html_slim,
    clean_html_with_profile, clean_html_with_profile_and_intent, smart_clean_html,
};
