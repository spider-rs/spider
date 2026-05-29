<p align="center">
  <a href="https://spider.cloud" target="_blank">
    <img src="https://avatars.githubusercontent.com/u/112983871?s=400&u=e03cc05523f015dd1f2a5ab9e6158de8a30821c2&v=4" alt="Spider" width="140" height="140">
  </a>
</p>

<h1 align="center">Spider</h1>

<p align="center">The fastest web crawler and scraper for Rust.</p>

<p align="center">
  <a href="https://crates.io/crates/spider"><img src="https://img.shields.io/crates/v/spider.svg" alt="Crates.io"></a>
  <a href="https://crates.io/crates/spider"><img src="https://img.shields.io/crates/d/spider.svg?label=downloads" alt="Downloads"></a>
  <a href="https://docs.rs/spider"><img src="https://docs.rs/spider/badge.svg" alt="Documentation"></a>
  <a href="https://discord.spider.cloud"><img src="https://img.shields.io/badge/discord-join-5865F2?logo=discord&logoColor=white" alt="Discord"></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-informational" alt="License"></a>
</p>

<h4 align="center">
  <a href="https://spider.cloud">spider.cloud</a> ·
  <a href="https://spider.cloud/guides">Guides</a> ·
  <a href="https://docs.rs/spider">Docs</a> ·
  <a href="./examples/">Examples</a> ·
  <a href="https://discord.spider.cloud">Discord</a>
</h4>

---

Spider is a concurrency-first crawling engine built in Rust. It streams pages the moment they arrive, renders JavaScript only when a page demands it, and scales from a single script to a distributed fleet without changing your code. The same engine powers [Spider Cloud](https://spider.cloud), so you can prototype locally and move to managed infrastructure with one config change.

## Start in the cloud

The hardest part of crawling at scale isn't the code. It's the proxies, headless browsers, and constant anti-bot churn. Spider Cloud runs all of that for you behind the same API.

[**Get a free API key →**](https://spider.cloud) (no card required)

```toml
[dependencies]
spider = { version = "2", features = ["spider_cloud"] }
```

```rust
use spider::configuration::{SpiderCloudConfig, SpiderCloudMode};
use spider::website::Website;

let cloud = SpiderCloudConfig::new("sk-...")
    .with_mode(SpiderCloudMode::Smart); // proxy by default, auto-unblock when blocked

let mut website = Website::new("https://example.com")
    .with_spider_cloud_config(cloud)
    .build()?;
```

`Smart` mode routes through proxies first and escalates to the unblocker only on pages that fight back, so you pay for bypass exactly when it's needed and never when it isn't.

## Or run it locally

No key, no service. Just the crawler.

```toml
[dependencies]
spider = "2"
```

```rust
use spider::{tokio, website::Website};

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://example.com");
    let mut rx = website.subscribe(16);

    tokio::spawn(async move {
        while let Ok(page) = rx.recv().await {
            println!("{}  {}", page.status_code, page.get_url());
        }
    });

    website.crawl().await;
    website.unsubscribe();
}
```

Pages stream in as they're fetched. The crawler discovers links, respects boundaries, and stops on its own.

## How it works

Spider runs HTTP-first and only launches headless Chrome when a page actually needs JavaScript. Streaming is built into both the HTTP and Chrome paths, so pages flow back the moment they're fetched instead of batching at the end. That design delivers best-in-class concurrency throughput, sustaining extremely high request volumes that scale from a single async task to a distributed worker fleet on the same API. Proxies, retries, rate limiting, and stealth are built in.

## Install

| You want… | Run |
|---|---|
| Rust library | `cargo add spider` |
| Command-line tool | `cargo install spider_cli` |
| Node.js package | `npm i @spider-rs/spider-rs` |
| Python package | `pip install spider_rs` |
| MCP server (Claude, Cursor, …) | `cargo install spider_mcp` |
| Managed crawling | [spider.cloud](https://spider.cloud) |

## Configuration

Every option has a sensible default, so set only what you need.

```rust
let mut website = Website::new("https://example.com")
    .with_limit(50)                    // concurrent requests
    .with_depth(10)                    // how deep to follow links
    .with_delay(500)                   // pause between requests (ms)
    .with_respect_robots_txt(true)
    .with_subdomains(true)
    .with_user_agent(Some("MyBot/1.0"))
    .with_stealth(true)
    .build()
    .unwrap();
```

Full reference in the [`Configuration` docs](https://docs.rs/spider/latest/spider/configuration/struct.Configuration.html).

For JavaScript-heavy sites, enable `features = ["chrome"]` and call `crawl_smart()`. Spider tries HTTP first and only launches Chrome on pages that need it.

## Use cases

Teams use Spider to feed the open web into vector stores for LLM and RAG pipelines, monitor sites for SEO and price changes, export pages as Markdown, JSON, or WARC, and drive headless Chrome for AI browsing agents. There are [50+ runnable examples](./examples/) to start from.

## Learn more

- 📚 [Guides](https://spider.cloud/guides): recipes and integrations
- 📖 [API docs](https://docs.rs/spider): every option and method
- 💬 [Discord](https://discord.spider.cloud): questions and ideas
- 🐛 [Issues](https://github.com/spider-rs/spider/issues): bugs and feature requests

## Contributing

PRs welcome. See [`CONTRIBUTING.md`](./CONTRIBUTING.md).

```bash
cargo test -p spider                  # unit tests
RUN_LIVE_TESTS=1 cargo test           # live network tests
```

## License

[MIT](./LICENSE).
