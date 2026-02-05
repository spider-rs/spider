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

Add spider to your project:

```toml
[dependencies]
spider = "2"
```

### Streaming Pages

Process pages as they're crawled with real-time subscriptions:

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
