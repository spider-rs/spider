<p align="center">
  <a href="https://spider.cloud?utm_source=github&utm_medium=readme&utm_campaign=spider_rs" target="_blank">
    <img src="assets/spider-mark.png" alt="Spider" width="140" height="140">
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
  <a href="https://spider.cloud?utm_source=github&utm_medium=readme&utm_campaign=spider_rs">spider.cloud</a> ·
  <a href="https://spider.cloud/guides?utm_source=github&utm_medium=readme&utm_campaign=spider_rs">Guides</a> ·
  <a href="https://docs.rs/spider">Docs</a> ·
  <a href="./examples/">Examples</a> ·
  <a href="https://discord.spider.cloud">Discord</a>
</h4>

---

Spider is a concurrency-first crawling engine written in Rust. It streams pages as they arrive, renders JavaScript only on pages that need it, and scales from a single script to a distributed fleet without changing your code. The same engine runs [Spider Cloud](https://spider.cloud?utm_source=github&utm_medium=readme&utm_campaign=spider_rs), so you can prototype locally and move to managed infrastructure with one config change.

## Start in the cloud

The hardest part of crawling at scale isn't the code. It's the proxies, headless browsers, and constant anti-bot churn. Spider Cloud runs all of that for you behind the same API.

[**Get a free API key →**](https://spider.cloud?utm_source=github&utm_medium=readme&utm_campaign=spider_rs) (no card required)

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

`Smart` mode routes through proxies first and escalates to the unblocker only on pages that fight back, so you pay for bypass only where it's needed.

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

Pages stream in as they're fetched. The crawler finds the links, stays inside the limits you set, and stops on its own.

## How it works

Spider runs HTTP-first and launches headless Chrome only when a page needs JavaScript. Both the HTTP and Chrome paths stream, so pages come back as they're fetched instead of batching at the end. The same API drives one async task or a distributed worker fleet, and the concurrency model doesn't change between them. Proxies, retries, rate limiting, and stealth are built in.

## Install

| You want… | Run |
|---|---|
| Rust library | `cargo add spider` |
| Command-line tool | `cargo install spider_cli` |
| Node.js package | `npm i @spider-rs/spider-rs` |
| Python package | `pip install spider_rs` |
| MCP server (Claude, Cursor, …) | `cargo install spider_mcp` |
| Managed crawling | [spider.cloud](https://spider.cloud?utm_source=github&utm_medium=readme&utm_campaign=spider_rs) |

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

- [Guides](https://spider.cloud/guides?utm_source=github&utm_medium=readme&utm_campaign=spider_rs) for recipes and integrations
- [API docs](https://docs.rs/spider) for every option and method
- [Discord](https://discord.spider.cloud) for questions and ideas
- [Issues](https://github.com/spider-rs/spider/issues) for bugs and feature requests

## Contributing

PRs welcome. See [`CONTRIBUTING.md`](./CONTRIBUTING.md).

```bash
cargo test -p spider                  # unit tests
RUN_LIVE_TESTS=1 cargo test           # live network tests
```

## License

[MIT](./LICENSE).
