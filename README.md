# Spider

[![Build Status](https://github.com/spider-rs/spider/actions/workflows/rust.yml/badge.svg)](https://github.com/spider-rs/spider/actions)
[![Crates.io](https://img.shields.io/crates/v/spider.svg)](https://crates.io/crates/spider)
[![Downloads](https://img.shields.io/crates/d/spider.svg)](https://crates.io/crates/spider)
[![Documentation](https://docs.rs/spider/badge.svg)](https://docs.rs/spider)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)
[![Discord](https://img.shields.io/discord/1254585814021832755.svg?logo=discord&style=flat-square)](https://discord.spider.cloud)

[Website](https://spider.cloud) |
[Guides](https://spider.cloud/guides) |
[API Docs](https://docs.rs/spider/latest/spider) |
[Examples](./examples/) |
[Discord](https://discord.spider.cloud)

A high-performance web crawler and scraper for Rust. One library for HTTP, headless Chrome, and WebDriver rendering, [200-1000x faster](#benchmarks) than popular crawlers in Go, Node.js, and C.

- **[200-1000x faster](#benchmarks)** than popular crawlers. Crawl 100k+ pages in minutes on a single machine.
- **One dependency** for HTTP, headless Chrome (CDP), WebDriver, and [AI-powered automation](./spider_agent/).
- **Production-ready** with caching, proxy rotation, anti-bot bypass, and [distributed crawling](./spider_worker/). Everything is [feature-gated](https://doc.rust-lang.org/cargo/reference/features.html) so you only compile what you use.

## Quick Start

### Command Line

```bash
cargo install spider_cli
spider --url https://example.com
```

### Rust

```toml
[dependencies]
spider = "2"
```

```rust
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://example.com");
    website.crawl().await;
    println!("Pages found: {}", website.get_links().len());
}
```

### Streaming

Process each page the moment it's crawled, not after:

```rust
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://example.com");
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

### Headless Chrome

Add one feature flag to render JavaScript-heavy pages:

```toml
[dependencies]
spider = { version = "2", features = ["chrome"] }
```

```rust
use spider::features::chrome_common::RequestInterceptConfiguration;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://example.com")
        .with_chrome_intercept(RequestInterceptConfiguration::new(true))
        .with_stealth(true)
        .build()
        .unwrap();

    website.crawl().await;
}
```

> Also supports [WebDriver](./examples/webdriver.rs) (Selenium Grid, remote browsers) and [AI-driven automation](./spider_agent/). See [examples](./examples/) for more.

## Benchmarks

Crawling 185 pages on `rsseau.fr` ([source](./benches/BENCHMARKS.md), 10 samples averaged):

**Apple M1 Max** (10-core, 64 GB RAM):

| Crawler | Language | Time | vs Spider |
|---------|----------|-----:|----------:|
| **spider** | **Rust** | **73 ms** | **baseline** |
| node-crawler | JavaScript | 15 s | 205x slower |
| colly | Go | 32 s | 438x slower |
| wget | C | 70 s | 959x slower |

**Linux** (2-core, 7 GB RAM):

| Crawler | Language | Time | vs Spider |
|---------|----------|-----:|----------:|
| **spider** | **Rust** | **50 ms** | **baseline** |
| node-crawler | JavaScript | 3.4 s | 68x slower |
| colly | Go | 30 s | 600x slower |
| wget | C | 60 s | 1200x slower |

The gap grows with site size. Spider handles 100k+ pages in minutes where other crawlers take hours. This comes from Rust's async runtime ([tokio](https://tokio.rs)), lock-free data structures, and optional [io_uring](https://en.wikipedia.org/wiki/Io_uring) on Linux. [Full details](./benches/BENCHMARKS.md)

## Why Spider?

Most crawlers force a choice between fast HTTP-only or slow-but-flexible browser automation. Spider supports both, and you can mix them in the same crawl.

**Supports HTTP, Chrome, and WebDriver.** Switch rendering modes with a feature flag. Use HTTP for speed, Chrome CDP for JavaScript-heavy pages, and WebDriver for Selenium Grid or cross-browser testing.

**Only compile what you use.** Every optional capability (Chrome, caching, proxies, AI) lives behind a [Cargo feature flag](https://doc.rust-lang.org/cargo/reference/features.html). A minimal `spider = "2"` stays lean.

**Built for production.** Caching (memory, disk, hybrid), proxy rotation, anti-bot fingerprinting, ad blocking, depth budgets, cron scheduling, and distributed workers. All of this has been hardened through [Spider Cloud](https://spider.cloud).

**AI automation included.** [spider_agent](./spider_agent/) adds multimodal LLM-driven automation: navigate pages, fill forms, solve challenges, and extract structured data with OpenAI or any compatible API.

## Features

<details>
<summary><strong>Crawling</strong></summary>

- Concurrent and streaming crawls with backpressure
- [Decentralized crawling](./spider_worker/) for horizontal scaling
- Caching: memory, disk (SQLite), or [hybrid Chrome cache](./examples/cache_chrome_hybrid.rs)
- Proxy support with rotation
- Cron job scheduling
- Depth budgeting, blacklisting, whitelisting
- Smart mode that auto-detects JS-rendered content and upgrades to Chrome

</details>

<details>
<summary><strong>Browser Automation</strong></summary>

- [Chrome DevTools Protocol](https://github.com/spider-rs/chromey): headless or headed, stealth mode, screenshots, request interception
- [WebDriver](./examples/webdriver.rs): Selenium Grid, remote browsers, cross-browser testing
- AI-powered challenge solving (deterministic + [Chrome built-in AI](https://developer.chrome.com/docs/ai/prompt-api))
- [Anti-bot fingerprinting](https://github.com/spider-rs/spider_fingerprint), [ad blocking](https://github.com/spider-rs/spider_network_blocker), [firewall](https://github.com/spider-rs/spider_firewall)

</details>

<details>
<summary><strong>Data Processing</strong></summary>

- [HTML transformations](https://github.com/spider-rs/spider_transformations) (Markdown, text, structured extraction)
- CSS/XPath scraping with [spider_utils](./spider_utils/README.md#CSS_Scraping)
- [OpenAI](./examples/openai.rs) and [Gemini](./examples/gemini.rs) integration for content analysis

</details>

<details>
<summary><strong>AI Agent</strong></summary>

- [spider_agent](./spider_agent/): concurrent-safe multimodal web automation agent
- Multiple LLM providers (OpenAI, any OpenAI-compatible API, Chrome built-in AI)
- Web research with search providers (Serper, Brave, Bing, Tavily)
- 110 built-in automation skills for web challenges

</details>

## Spider Cloud

For managed proxy rotation, anti-bot bypass, and CAPTCHA handling, [Spider Cloud](https://spider.cloud) plugs in with one line:

```rust
let mut website = Website::new("https://protected-site.com")
    .with_spider_cloud("your-api-key")  // enable with features = ["spider_cloud"]
    .build()
    .unwrap();
```

| Mode | Strategy | Best For |
|------|----------|----------|
| **Proxy** (default) | All traffic through Spider Cloud proxy | General crawling with IP rotation |
| **Smart** (recommended) | Proxy + auto-fallback on bot detection | Production (speed + reliability) |
| **Fallback** | Direct first, API on failure | Cost-efficient, most sites work without help |
| **Unblocker** | All requests through unblocker | Aggressive bot protection |

> Free credits on signup. [Get started at spider.cloud](https://spider.cloud)

## Get Spider

| Package | Language | Install |
|---------|----------|---------|
| [spider](https://crates.io/crates/spider) | Rust | `cargo add spider` |
| [spider_cli](./spider_cli/) | CLI | `cargo install spider_cli` |
| [spider-nodejs](https://github.com/spider-rs/spider-nodejs) | Node.js | `npm i @spider-rs/spider-rs` |
| [spider-py](https://github.com/spider-rs/spider-py) | Python | `pip install spider_rs` |
| [spider_agent](./spider_agent/) | Rust | `cargo add spider --features agent` |
| [Spider Cloud](https://spider.cloud) | API | Managed infrastructure, no install needed |

## Resources

- [64 examples](./examples/) covering crawling, Chrome, WebDriver, AI, caching, and more
- [API documentation](https://docs.rs/spider/latest/spider)
- [Benchmarks](./benches/BENCHMARKS.md)
- [Changelog](CHANGELOG.md)

## Contributing

Contributions welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and guidelines.

Spider has been actively developed since 2018. Join the [Discord](https://discord.spider.cloud) for questions and discussion.

## License

[MIT](LICENSE)
