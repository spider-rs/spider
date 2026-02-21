# Spider

[![Build Status](https://github.com/spider-rs/spider/actions/workflows/rust.yml/badge.svg)](https://github.com/spider-rs/spider/actions)
[![Crates.io](https://img.shields.io/crates/v/spider.svg)](https://crates.io/crates/spider)
[![Documentation](https://docs.rs/spider/badge.svg)](https://docs.rs/spider)
[![Rust](https://img.shields.io/badge/rust-1.56.1%2B-blue.svg?maxAge=3600)](https://github.com/spider-rs/spider)
[![Discord chat](https://img.shields.io/discord/1254585814021832755.svg?logo=discord&style=flat-square)](https://discord.spider.cloud)

[Website](https://spider.cloud) |
[Guides](https://spider.cloud/guides) |
[API Docs](https://docs.rs/spider/latest/spider) |
[Chat](https://discord.spider.cloud)

A web crawler and scraper written in Rust.

- Concurrent crawling with streaming
- HTTP, Chrome (CDP), or WebDriver rendering
- Caching, proxies, and distributed crawling

## Features

### Core
- Concurrent & streaming crawls
- [Decentralized crawling](./spider_worker/) for horizontal scaling
- Caching (memory, disk, or hybrid)
- Proxy support with rotation
- Cron job scheduling

### Browser Automation
- [Chrome DevTools Protocol (CDP)](https://github.com/spider-rs/chromey) for local Chrome
- **WebDriver** support for Selenium Grid, remote browsers, and cross-browser testing
- AI-powered automation workflows
- Web challenge solving (deterministic + [AI built-in](https://developer.chrome.com/docs/ai/prompt-api))

### Data Processing
- [HTML transformations](https://github.com/spider-rs/spider_transformations)
- CSS/XPath scraping with [spider_utils](./spider_utils/README.md#CSS_Scraping)
- Smart mode for JS-rendered content detection

### Security & Control
- [Anti-bot mitigation](https://github.com/spider-rs/spider_fingerprint)
- [Ad blocking](https://github.com/spider-rs/spider_network_blocker)
- [Firewall](https://github.com/spider-rs/spider_firewall)
- Blacklisting, whitelisting, and depth budgeting
- [Spider Cloud](https://spider.cloud) integration for proxy rotation and anti-bot bypass (`spider_cloud` feature)

### AI Agent
- [spider_agent](./spider_agent/) - Concurrent-safe multimodal agent for web automation and research
- Multiple LLM providers (OpenAI, OpenAI-compatible APIs)
- Multiple search providers (Serper, Brave, Bing, Tavily)
- HTML extraction and research synthesis

## Quick Start

```toml
[dependencies]
spider = "2"
```

Crawl a website in three lines:

```rust
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://spider.cloud");
    website.crawl().await;
    println!("Pages found: {}", website.get_links().len());
}
```

### Streaming Pages

Process pages as they're crawled in real time:

```rust
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://spider.cloud");
    let mut rx = website.subscribe(0).unwrap();

    tokio::spawn(async move {
        while let Ok(page) = rx.recv().await {
            println!("- {}", page.get_url());
        }
    });

    website.crawl().await;
    website.unsubscribe();
}
```

### Chrome (CDP)

Render JavaScript-heavy pages with stealth mode and request interception:

```toml
[dependencies]
spider = { version = "2", features = ["chrome"] }
```

```rust
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://spider.cloud")
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true)
        .build()
        .unwrap();

    website.crawl().await;
}
```

### WebDriver (Selenium Grid)

Connect to remote browsers, Selenium Grid, or any W3C WebDriver-compatible service:

```toml
[dependencies]
spider = { version = "2", features = ["webdriver"] }
```

```rust
use spider::features::webdriver_common::{WebDriverConfig, WebDriverBrowser};
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://spider.cloud")
        .with_webdriver(
            WebDriverConfig::new()
                .with_server_url("http://localhost:4444")
                .with_browser(WebDriverBrowser::Chrome)
                .with_headless(true)
        )
        .build()
        .unwrap();

    website.crawl().await;
}
```

## Spider Cloud: Reliable Crawling at Scale

Production crawling means dealing with bot protection, CAPTCHAs, rate limits, and blocked requests. **Spider Cloud** integration adds a reliability layer that handles all of this automatically — no code changes required beyond adding your API key.

> **New to Spider Cloud?** [Sign up at spider.cloud](https://spider.cloud) to get your API key. New accounts receive free credits so you can try it out before committing.

Enable the feature:

```toml
[dependencies]
spider = { version = "2", features = ["spider_cloud"] }
```

### How It Works

When you provide a Spider Cloud API key, your crawler gains access to:

- **Managed proxy rotation** — requests route through `proxy.spider.cloud` with automatic IP rotation, geo-targeting, and residential proxies
- **Anti-bot bypass** — Cloudflare, Akamai, Imperva, Distil Networks, and generic CAPTCHA challenges are handled transparently
- **Automatic fallback** — if a direct request fails (403, 429, 503, 5xx), the request is retried through Spider Cloud's unblocking infrastructure
- **Content-aware detection** — Smart mode inspects response bodies for challenge pages, empty responses, and bot detection markers before you ever see them

### Integration Modes

Choose the mode that fits your workload:

| Mode | Strategy | Best For |
|------|----------|----------|
| **Proxy** (default) | Route all traffic through Spider Cloud proxy | General crawling with proxy rotation |
| **Smart** (recommended) | Proxy by default, auto-fallback to unblocker on bot detection | Production workloads — best balance of speed and reliability |
| **Fallback** | Direct fetch first, fall back to API on failure | Cost-efficient crawling where most sites work without help |
| **Unblocker** | All requests through the unblocker API | Sites with aggressive bot protection |
| **Api** | All requests through the crawl API | Simple scraping, one page at a time |

**Smart mode** is the recommended choice for production. It detects and handles:
- HTTP 403, 429, 503, and Cloudflare 520-530 errors
- Cloudflare browser verification challenges
- CAPTCHA and "verify you are human" pages
- Distil Networks, Imperva, and Akamai Bot Manager
- Empty response bodies on HTML pages

### Quick Setup

One line to enable proxy routing:

```rust
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://example.com")
        .with_spider_cloud("your-api-key")  // Proxy mode (default)
        .build()
        .unwrap();

    website.crawl().await;
}
```

### Smart Mode (Recommended)

For production, use Smart mode to get automatic fallback when pages are protected:

```rust
use spider::configuration::{SpiderCloudConfig, SpiderCloudMode};
use spider::website::Website;

#[tokio::main]
async fn main() {
    let config = SpiderCloudConfig::new("your-api-key")
        .with_mode(SpiderCloudMode::Smart);

    let mut website = Website::new("https://protected-site.com")
        .with_spider_cloud_config(config)
        .build()
        .unwrap();

    website.crawl().await;
}
```

What happens under the hood in Smart mode:

1. Request goes through `proxy.spider.cloud` (fast, low cost)
2. If the response is a 403/429/503, a challenge page, or an empty body → automatic retry through the `/unblocker` API
3. The unblocked content is returned transparently — your code sees a normal page

### CLI Usage

```bash
spider --url https://example.com \
  --spider-cloud-key "your-api-key" \
  --spider-cloud-mode smart
```

### Extra Parameters

Pass additional options to the Spider Cloud API for fine-grained control:

```rust
use spider::configuration::{SpiderCloudConfig, SpiderCloudMode};

let mut params = hashbrown::HashMap::new();
params.insert("stealth".into(), serde_json::json!(true));
params.insert("fingerprint".into(), serde_json::json!(true));

let config = SpiderCloudConfig::new("your-api-key")
    .with_mode(SpiderCloudMode::Smart)
    .with_extra_params(params);
```

> Get started at [spider.cloud](https://spider.cloud) — new signups receive free credits to test the full integration.

## Get Spider

| Method | Best For |
|--------|----------|
| [Spider Cloud](https://spider.cloud) | Production workloads, no setup required |
| [spider](./spider/README.md) | Rust applications |
| [spider_agent](./spider_agent/README.md) | AI-powered web automation and research |
| [spider_cli](./spider_cli/README.md) | Command-line usage |
| [spider-nodejs](https://github.com/spider-rs/spider-nodejs) | Node.js projects |
| [spider-py](https://github.com/spider-rs/spider-py) | Python projects |

## Resources

- [Examples](./examples/) - Code samples for common use cases
- [Benchmarks](./benches/BENCHMARKS.md) - Performance comparisons
- [Changelog](CHANGELOG.md) - Version history

## License

[MIT](https://github.com/spider-rs/spider/blob/main/LICENSE)

## Contributing

See [CONTRIBUTING](CONTRIBUTING.md).
