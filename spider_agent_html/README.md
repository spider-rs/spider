# Spider Agent HTML

Streaming HTML processing utilities for `spider_agent` — cleaning, content-aware profile selection, and intent-based optimization.

## Overview

This crate provides fast, single-pass HTML cleaning using `lol_html`'s streaming rewriter. No full DOM parsing required — O(n) processing with constant memory overhead.

## Installation

```toml
[dependencies]
spider_agent_html = "0.1"
```

## Quick Start

```rust
use spider_agent_html::{clean_html_base, clean_html_slim, smart_clean_html};

let html = r#"<html><head><script>tracker();</script><style>body{}</style></head>
<body><h1>Hello</h1><p>World</p><svg>...</svg></body></html>"#;

// Base: remove scripts, styles, ads, tracking
let clean = clean_html_base(html);

// Slim: also remove SVG, canvas, video, base64
let slim = clean_html_slim(html);

// Smart: auto-select the optimal profile based on content analysis
let smart = smart_clean_html(html);
```

## Cleaning Profiles

| Profile | Removes | Use Case |
|---------|---------|----------|
| **Raw** | Nothing | Full HTML preservation |
| **Minimal** | `<script>`, `<style>` | Visual pages, screenshots |
| **Default** | Scripts, styles, ads, tracking, meta | General-purpose |
| **Slim** | Default + SVG, canvas, video, base64 | Token-conscious LLM input |
| **Aggressive** | Everything non-text | Maximum token reduction |

```rust
use spider_agent_html::clean_html_with_profile;
use spider_agent_types::HtmlCleaningProfile;

let result = clean_html_with_profile(html, HtmlCleaningProfile::Slim);
```

## Smart Cleaning

`smart_clean_html()` runs content analysis first, then picks the lightest profile that achieves good token reduction:

```rust
use spider_agent_html::smart_clean_html;

// Automatically picks Slim for SVG-heavy pages, Base for simple pages, etc.
let cleaned = smart_clean_html(large_html);
```

## Intent-Based Cleaning

Optimize cleaning for the downstream task:

```rust
use spider_agent_html::clean_html_with_profile_and_intent;
use spider_agent_types::{HtmlCleaningProfile, CleaningIntent};

// More aggressive cleaning for extraction tasks
let result = clean_html_with_profile_and_intent(
    html,
    HtmlCleaningProfile::Default,
    CleaningIntent::Extraction,
);
```

## API Reference

| Function | Description |
|----------|-------------|
| `clean_html(html)` | Default profile cleaning |
| `clean_html_raw(html)` | Passthrough, no cleaning |
| `clean_html_base(html)` | Remove scripts, styles, ads, tracking |
| `clean_html_slim(html)` | Base + heavy media elements |
| `clean_html_full(html)` | Aggressive, text-only extraction |
| `clean_html_with_profile(html, profile)` | Apply specific profile |
| `clean_html_with_profile_and_intent(html, profile, intent)` | Profile + intent optimization |
| `smart_clean_html(html)` | Auto-select optimal profile |

## Dependencies

| Crate | Purpose |
|-------|---------|
| `lol_html` | Streaming HTML rewriter (Cloudflare) |
| `aho-corasick` | Fast multi-pattern matching |
| `serde` + `serde_json` | Serialization |
| `spider_agent_types` | Type definitions and content analysis |

## License

MIT
