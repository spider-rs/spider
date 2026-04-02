# Spider - Development Guide

The fastest web crawler and scraper for Rust. Workspace: `spider`, `spider_cli`, `spider_agent`, `spider_agent_types`, `spider_agent_html`, `spider_utils`, `spider_worker`, `spider_mcp`.

## Quick Reference

```
docs.rs/spider          # API docs
spider.cloud            # Website & managed cloud
spider.cloud/guides     # Guides
discord.spider.cloud    # Community
```

## Getting Started

```toml
[dependencies]
spider = { version = "2", features = ["spider_cloud"] }
```

### Spider Cloud (recommended)

Sign up at <https://spider.cloud> for a free API key, then:

```rust
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://example.com");
    website.with_spider_cloud("YOUR_API_KEY");

    let mut rx = website.subscribe(0).unwrap();
    tokio::spawn(async move {
        while let Ok(page) = rx.recv().await {
            println!("{} - {} bytes", page.get_url(), page.get_html_bytes_u8().len());
        }
    });

    website.crawl().await;
    website.unsubscribe();
}
```

### Local crawl (no API key)

```rust
let mut website = Website::new("https://example.com");
website.crawl().await;
for link in website.get_links() {
    println!("{link}");
}
```

---

## Spider Cloud

Spider Cloud offloads crawling to managed infrastructure with anti-bot bypass, proxies, and browser rendering. All Spider Cloud features require `features = ["spider_cloud"]`.

### Cloud Modes (`SpiderCloudMode`)

| Mode | Description | When to use |
|------|-------------|-------------|
| **Proxy** (default) | Routes HTTP through `proxy.spider.cloud` | General crawling, transparent |
| **Api** | `POST /crawl` per page | Need API-level control |
| **Unblocker** | `POST /unblocker` per page | Sites with heavy bot protection |
| **Fallback** | Direct fetch first, cloud on 403/429/503 | Cost-conscious, mostly unprotected sites |
| **Smart** | Proxy default, auto-fallback to `/unblocker` on bot detection | **Production recommended** |

### Configuration

```rust
use spider::configuration::{SpiderCloudConfig, SpiderCloudMode};

// Full control via SpiderCloudConfig
let config = SpiderCloudConfig::new("sk-...")
    .with_mode(SpiderCloudMode::Smart)       // proxy + auto unblocker fallback
    .with_return_format("raw");              // "raw" = original HTML

let mut website = Website::new("https://example.com")
    .with_spider_cloud_config(config)
    .build()
    .unwrap();

// Or shorthand — defaults to Proxy mode
let mut website = Website::new("https://example.com");
website.with_spider_cloud("sk-...");
```

**Smart mode** auto-detects bot protection via status codes (403, 429, 503, 520-530) and content markers (Cloudflare challenge, CAPTCHA, Distil, Imperva, Akamai). When detected, it falls back from proxy to `/unblocker` API automatically.

### Browser Cloud (remote headless Chrome via CDP)

Requires `features = ["spider_cloud", "chrome"]` (both).

```rust
use spider::configuration::SpiderBrowserConfig;

let browser = SpiderBrowserConfig::new("sk-...")
    .with_stealth(true)          // anti-fingerprinting
    .with_country("us");         // geo-targeting

let mut website = Website::new("https://example.com")
    .with_limit(10)
    .with_spider_browser_config(browser)
    .build()
    .unwrap();

website.crawl().await;
```

Connects via `wss://browser.spider.cloud/v1/browser`. Use `.connection_url()` on `SpiderBrowserConfig` to see the full URL with auth params.

### CLI

```bash
# Store API key
spider authenticate sk-...

# Crawl with cloud
spider crawl --url https://example.com --spider-cloud-mode smart

# Browser cloud
spider crawl --url https://example.com --spider-cloud-browser
```

**Key resolution order:** `--spider-cloud-key` flag > `SPIDER_CLOUD_API_KEY` env > `~/.spider/credentials`

### Environment Variables

| Variable | Purpose |
|----------|---------|
| `SPIDER_CLOUD_API_KEY` | API key |
| `SPIDER_CLOUD_API_URL` | Custom API URL (default: `https://api.spider.cloud`) |
| `SPIDER_CLOUD_RETURN_FORMAT` | `raw\|markdown\|commonmark\|text\|bytes` |
| `SPIDER_CLOUD_FORCE_UNBLOCKER` | Always use unblocker (`1`/`true`) |
| `SPIDER_BROWSER_STEALTH` | Enable stealth mode (`1`/`true`) |
| `SPIDER_BROWSER_COUNTRY` | Country code (e.g. `us`, `gb`) |

---

## Core Configuration

`Configuration` in `spider/src/configuration.rs` — all fields have sensible defaults.

### Builder Pattern

```rust
use spider::website::Website;

let mut website = Website::new("https://example.com")
    .with_limit(50)                                    // concurrency
    .with_depth(10)                                    // max link depth
    .with_delay(500)                                   // ms between requests
    .with_request_timeout(Some(Duration::from_secs(30)))
    .with_respect_robots_txt(true)
    .with_subdomains(true)
    .with_user_agent(Some("MyBot/1.0"))
    .with_blacklist_url(Some(vec!["/admin".into()]))
    .with_whitelist_url(Some(vec!["/blog".into()]))
    .with_headers(Some(headers))
    .with_proxies(Some(vec!["http://proxy:8080".into()]))
    .with_external_domains(Some(vec!["https://cdn.example.com".to_string()].into_iter()))
    .with_budget(Some(HashMap::from([("/blog", 100)])))
    .with_caching(true)
    .with_stealth(true)
    .build()
    .unwrap();
```

### Chrome Rendering

Requires `features = ["chrome"]`. Use the [headless-browser](https://github.com/spider-rs/headless-browser) Docker image or local `chrome-headless-shell`.

```rust
// features = ["chrome"]
use spider::configuration::RequestInterceptConfiguration;

let mut website = Website::new("https://spa-app.com")
    .with_chrome_intercept(RequestInterceptConfiguration::new(true))  // block ads/analytics/stylesheets
    .with_stealth(true)
    .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(30)))))
    .build()
    .unwrap();
website.crawl().await;
```

For remote Chrome: `.with_chrome_connection(Some("ws://localhost:9222".into()))`

Related features: `chrome_headed` (visible window), `chrome_stealth` (anti-fingerprinting), `chrome_screenshot` (page capture), `chrome_intercept` (network blocking), `chrome_headless_new` (uses `--headless=new`), `chrome_cpu` (disable GPU)

### Smart Mode

Starts with HTTP, upgrades to Chrome only when JavaScript rendering is detected.

```rust
// features = ["smart"]
let mut website = Website::new("https://example.com")
    .build()
    .unwrap();
website.crawl_smart().await;
```

### Subscription (streaming pages)

Requires `features = ["sync"]` (included in `basic`).

```rust
let mut website = Website::new("https://example.com");
let mut rx = website.subscribe(16).unwrap();

tokio::spawn(async move {
    while let Ok(page) = rx.recv().await {
        let url = page.get_url();
        let html = page.get_html();
        let status = page.status_code;
    }
});

website.crawl().await;
website.unsubscribe();
```

### Wait Conditions

Requires `features = ["chrome"]`. Use the `with_wait_for_*` methods on `Website` (or `Configuration`):

```rust
use spider::configuration::{WaitForIdleNetwork, WaitForSelector, WaitForDelay};  // re-exported from chrome_common
use std::time::Duration;

let mut website = Website::new("https://example.com")
    // Wait for network idle (500ms no connections)
    .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(30)))))
    // Wait for a CSS selector to appear
    .with_wait_for_selector(Some(WaitForSelector::new(
        Some(Duration::from_secs(10)),
        "div.loaded".into(),
    )))
    // Wait with a fixed delay (testing only)
    .with_wait_for_delay(Some(WaitForDelay::new(Some(Duration::from_millis(2000)))))
    .build()
    .unwrap();
```

There is also `with_wait_for_idle_network0` (waits for zero outstanding requests) and `with_wait_for_idle_dom` (waits for DOM mutations to stop on a selector).

---

## Feature Flags

### Essentials

| Feature | What it does |
|---------|-------------|
| `basic` | Full default crawling (cookies, UA, encoding, caching, rate limiting, etc.) |
| `spider_cloud` | Spider Cloud integration |
| `chrome` | Headless Chrome rendering |
| `smart` | Hybrid HTTP + Chrome |
| `serde` | Serialization support |
| `sync` | Broadcast channels for page streaming |

### Chrome Variants

`chrome_headed`, `chrome_stealth`, `chrome_screenshot`, `chrome_intercept`, `chrome_headless_new`, `chrome_cpu`, `chrome_remote_cache`

### Caching

| Feature | Backend |
|---------|---------|
| `cache` | Disk (cacache) |
| `cache_mem` | In-memory |
| `cache_chrome_hybrid` | Chrome + HTTP to disk |
| `cache_openai` / `cache_gemini` | LLM response caching |
| `etag_cache` | ETag-based cache |

### Networking

`socks`, `reqwest_rustls_tls`, `reqwest_native_tls`, `reqwest_hickory_dns`, `h2_multiplex`, `wreq`

### Performance

`io_uring` (Linux, default), `tcp_fastopen`, `splice`, `numa`, `zero_copy`, `simd`, `inline-more`, `bloom`

### AI / LLM

`openai`, `gemini`, `agent`, `agent_chrome`, `agent_skills`, `agent_full`

### Advanced

`rate_limit`, `adaptive_concurrency`, `request_coalesce`, `auto_throttle`, `priority_frontier`, `hedge`, `parallel_backends`, `firewall`, `cron`, `sitemap`, `webdriver`, `decentralized`, `disk`

---

## Spider Agent

AI-powered autonomous browsing agent. Located in `spider_agent/`.

### With Spider Cloud Tools

```rust
use spider_agent::{Agent, SpiderCloudToolConfig};

// Full config
let agent = Agent::builder()
    .with_spider_cloud_config(
        SpiderCloudToolConfig::new("sk-...")
            .with_enable_ai_routes(true)  // paid plan — enables /ai/* routes
    )
    .build()?;

// Or shorthand (defaults)
let agent = Agent::builder()
    .with_spider_cloud("sk-...")
    .build()?;

// Available tools: spider_cloud_crawl, spider_cloud_scrape,
// spider_cloud_search, spider_cloud_links, spider_cloud_transform,
// spider_cloud_unblocker
// AI tools (paid): spider_cloud_ai_crawl, spider_cloud_ai_scrape,
// spider_cloud_ai_search, spider_cloud_ai_browser, spider_cloud_ai_links
```

### With Browser Cloud Tools

```rust
use spider_agent::{Agent, SpiderBrowserToolConfig};

// Full config
let agent = Agent::builder()
    .with_spider_browser_config(
        SpiderBrowserToolConfig::new("sk-...")
            .with_stealth(true)
            .with_country("us")
    )
    .build()?;

// Or shorthand
let agent = Agent::builder()
    .with_spider_browser("sk-...")
    .build()?;

// Available tools: spider_browser_navigate, spider_browser_html,
// spider_browser_screenshot, spider_browser_evaluate,
// spider_browser_click, spider_browser_fill, spider_browser_wait
```

### Agent Examples

```bash
# End-to-end pipeline
SPIDER_CLOUD_API_KEY=sk-... cargo run -p spider_agent --example spider_cloud_end_to_end \
  -- "Find top travel books on https://books.toscrape.com"

# Prompt-driven flows (crawl, scrape, search, transform, unblocker)
SPIDER_CLOUD_API_KEY=sk-... cargo run -p spider_agent --example spider_cloud_prompt_flows \
  -- "run all flows for https://books.toscrape.com/"

# Jobs pipeline
SPIDER_CLOUD_API_KEY=sk-... cargo run -p spider_agent --example spider_cloud_jobs_pipeline \
  -- "rust engineer remote" "https://remoteok.com/remote-rust-jobs"

# Browser cloud
SPIDER_CLOUD_API_KEY=sk-... cargo run -p spider_agent --example spider_browser_cloud
```

---

## Running Examples

```bash
git clone https://github.com/spider-rs/spider.git && cd spider

# Basic crawl
cargo run --example example

# With Chrome
cargo run --example chrome --features chrome

# Smart mode
cargo run --example smart --features smart

# Spider Cloud (browser)
SPIDER_CLOUD_API_KEY=sk-... cargo run --example spider_browser_cloud --features "chrome spider_cloud"

# AI automation (OpenAI vision)
OPENAI_API_KEY=sk-... cargo run --example openai --features "chrome openai"

# Remote multimodal (any LLM)
cargo run --example remote_multimodal --features "chrome openai"

# Advanced configuration (reusable config across sites)
cargo run --example advanced_configuration

# Anti-bot / stealth
cargo run --example anti_bots --features "chrome chrome_stealth"

# Cache + Chrome hybrid
cargo run --example cache_chrome_hybrid --features "cache_chrome_hybrid chrome"

# See all examples (48+ files)
ls examples/
```

Use `--release` for production-level performance.

Full examples list in [examples/README.md](./examples/README.md).

---

## Testing

### Unit tests

```bash
cargo test -p spider
cargo test -p spider_agent
```

### Live integration tests

Require network access and optional API keys:

```bash
# Core crawler tests against crawler-test.com
RUN_LIVE_TESTS=1 cargo test -p spider --test crawler_test_com

# Spider Cloud integration
SPIDER_CLOUD_API_KEY=sk-... RUN_LIVE_TESTS=1 cargo test -p spider_agent --test live_spider_cloud

# Spider Browser integration
SPIDER_CLOUD_API_KEY=sk-... RUN_LIVE_TESTS=1 cargo test -p spider_agent --test live_spider_browser

# Agent cloud integration
SPIDER_CLOUD_API_KEY=sk-... RUN_LIVE_TESTS=1 cargo test -p spider --test live_spider_agent_cloud
```

### Feature-specific tests

```bash
# Smart vs Chrome comparison
cargo test -p spider --test smart_vs_chrome --features "smart chrome"

# Parallel backends
cargo test -p spider --test parallel_backends --features parallel_backends

# io_uring
cargo test -p spider uring_fs --features io_uring
```

---

## CLI

```bash
cargo install spider_cli

# Basic crawl
spider crawl --url https://example.com

# With Spider Cloud
spider authenticate sk-...
spider crawl --url https://example.com --spider-cloud-mode smart

# Browser cloud
spider crawl --url https://example.com --spider-cloud-browser

# Scrape
spider scrape --url https://example.com

# Download
spider download --url https://example.com
```

---

## Key Architecture Notes

- `spider/src/configuration.rs` — All config structs (`Configuration`, `SpiderCloudConfig`, `SpiderBrowserConfig`)
- `spider/src/website.rs` — Core `Website` struct with crawl methods
- `spider/src/page.rs` — `Page` struct (URL, HTML, status, headers, metadata)
- `spider/src/features/` — Feature-gated modules (chrome, webdriver, openai, etc.)
- `spider_agent/src/tools.rs` — Spider Cloud + Browser tool configs for agents
- `spider_agent/src/agent.rs` — Agent builder
- `spider_cli/src/main.rs` — CLI entry point and auth handling
- `spider_cli/src/options/args.rs` — CLI flags

### Crate dependency order

`spider_agent_types` -> `spider_agent_html` -> `spider_agent` -> `spider` -> `spider_cli` / `spider_utils` / `spider_worker` / `spider_mcp`

### Publishing

```bash
# Release script publishes in dependency order (--no-verify for cli/utils/worker)
./release.sh
```

---

## Common Patterns

### Reusable config across multiple sites

```rust
use spider::{configuration::Configuration, website::Website};

let config = Configuration::new()
    .with_user_agent(Some("MyBot/1.0"))
    .with_respect_robots_txt(true)
    .with_subdomains(false)
    .build();

for url in ["https://a.com", "https://b.com", "https://c.com"] {
    match Website::new(url).with_config(config.to_owned()).build() {
        Ok(mut site) => {
            site.crawl().await;
            let links = site.get_all_links_visited().await;
            println!("{url}: {} pages", links.len());
        }
        Err(e) => println!("Invalid URL: {:?}", e.get_url()),
    }
}
```

### Collect pages into a Vec

```rust
let mut website = Website::new("https://example.com");
website.crawl().await;
if let Some(pages) = website.get_pages() {
    for page in pages {
        println!("{}: {} bytes", page.get_url(), page.get_html_bytes_u8().len());
    }
}
```

### Custom HTTP client headers

```rust
use spider::reqwest::header::{HeaderMap, HeaderValue};

let mut headers = HeaderMap::new();
headers.insert("X-Custom", HeaderValue::from_static("value"));

let mut website = Website::new("https://example.com")
    .with_headers(Some(headers))
    .build()
    .unwrap();
```

### Budget limiting

```rust
use std::collections::HashMap;

let mut website = Website::new("https://example.com")
    .with_budget(Some(HashMap::from([
        ("/blog", 50),   // max 50 pages under /blog
        ("*", 200),      // max 200 pages total
    ])))
    .build()
    .unwrap();
```

### Screenshots

```rust
// features = ["chrome", "chrome_screenshot"]
use spider::configuration::{ScreenShotConfig, ScreenshotParams};

let screenshot_params = ScreenshotParams::new(Default::default(), Some(true), Some(true));
// ScreenShotConfig::new(params, bytes, save, output_dir)
let screenshot_config = ScreenShotConfig::new(screenshot_params, true, true, None);

let mut website = Website::new("https://example.com")
    .with_screenshot(Some(screenshot_config))
    .build()
    .unwrap();
```

### OpenAI integration

```rust
// features = ["chrome", "openai"]
use spider::configuration::GPTConfigs;

let gpt_config = GPTConfigs::new("gpt-4o", "Extract the main content", 512);

let mut website = Website::new("https://example.com")
    .with_chrome_intercept(RequestInterceptConfiguration::new(true))
    .with_wait_for_idle_network(Some(WaitForIdleNetwork::new(Some(Duration::from_secs(30)))))
    .with_screenshot(Some(screenshot_config))
    .with_openai(Some(gpt_config))
    .build()
    .unwrap();
```

### Cron scheduled crawl

```rust
// features = ["cron"]
let mut config = Configuration::new();
config.cron_str = "0 */6 * * *".into(); // every 6 hours (https://crontab.guru)
```

### WARC output

```rust
// features = ["warc"]
use spider::utils::warc::WarcConfig;

let mut config = Configuration::new();
config.warc = Some(WarcConfig {
    path: "output.warc.gz".to_string(),
    ..Default::default()
});
```
