# spider_agent_html — Skills & Capabilities

Streaming HTML processing utilities for spider_agent. Uses `lol_html` for fast, single-pass rewriting — no full DOM parsing required.

## Dependencies

`lol_html` + `aho-corasick` + `serde` + `serde_json` + `spider_agent_types`

---

## HTML Cleaning Functions

| Export | Signature | Description |
|--------|-----------|-------------|
| `clean_html_raw` | `(html: &str) -> String` | Passthrough — no cleaning, returns input unchanged |
| `clean_html_base` | `(html: &str) -> String` | Removes `<script>`, `<style>`, `<link>`, `<iframe>`, ads, tracking pixels, non-essential `<meta>` tags |
| `clean_html_slim` | `(html: &str) -> String` | Base + removes `<svg>`, `<noscript>`, `<canvas>`, `<video>`, base64 images |
| `clean_html_full` | `(html: &str) -> String` | Aggressive cleaning — strips everything non-essential for text extraction |
| `clean_html` | `(html: &str) -> String` | Default profile cleaning (equivalent to base) |
| `clean_html_with_profile` | `(html: &str, profile: HtmlCleaningProfile) -> String` | Apply a specific cleaning profile: Raw, Minimal, Default, Slim, Aggressive |
| `clean_html_with_profile_and_intent` | `(html: &str, profile: HtmlCleaningProfile, intent: CleaningIntent) -> String` | Profile + intent-based cleaning for goal-specific optimization |
| `smart_clean_html` | `(html: &str) -> String` | Auto-selects profile based on `ContentAnalysis` — picks the lightest profile that achieves good token reduction |

## Cleaning Profiles

| Profile | What it removes | Use case |
|---------|----------------|----------|
| **Raw** | Nothing | When you need the original HTML |
| **Minimal** | Only `<script>` and `<style>` | Visual inspection, screenshot pages |
| **Default** | Scripts, styles, ads, tracking, meta | General-purpose cleaning |
| **Slim** | Default + SVG, canvas, video, base64 | Token-conscious LLM input |
| **Aggressive** | Everything non-text | Maximum token reduction for extraction |

## Architecture

- **Streaming rewriter**: Uses `lol_html::rewrite_str()` with element content handlers — O(n) single-pass, no DOM tree allocation
- **Smart analysis**: `smart_clean_html()` first runs `ContentAnalysis::analyze()` (from `spider_agent_types`) to measure text ratio, SVG/script/base64 byte counts, then picks the optimal profile
- **Intent-aware**: `clean_html_with_profile_and_intent()` adjusts cleaning based on the downstream goal (extraction vs. observation vs. mapping)
- **Composable**: All functions are pure `&str -> String` — easy to chain, test, and benchmark
