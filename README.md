<p align="center">
  <a href="https://spider.cloud" target="_blank">
    <img src="https://avatars.githubusercontent.com/u/112983871?s=400&u=e03cc05523f015dd1f2a5ab9e6158de8a30821c2&v=4" alt="Spider" width="140" height="140">
  </a>
</p>

<h1 align="center">Spider</h1>

<p align="center">A production-grade web crawler and scraper for Rust.</p>

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

Spider is an open-source web crawler and scraper for Rust. Point it at a URL and you get the pages back as a stream — no batching, no waiting for the crawl to finish.

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

The whole program. Pages stream as they arrive. The crawler stops on its own.

## What's inside

- ⚡ Built for speed — handles large crawls without falling over
- 🌊 Streams pages as they're fetched
- 🧠 Renders JavaScript only when a page needs it
- 🛡️ Proxies, retries, and stealth built in
- 🧩 Scales from one script to a fleet of workers — same API
- 🦀 Pure Rust, embeddable anywhere

## Don't want to run the plumbing?

Crawling the modern web takes ongoing work — proxy pools, headless Chrome, Cloudflare updates, fingerprinting. We maintain all of that as a service so you don't have to.

[Spider Cloud](https://spider.cloud) is a managed backend for the same crawler. The code stays the same; the operational load goes away.

```rust
use spider::configuration::{SpiderCloudConfig, SpiderCloudMode};

let cloud = SpiderCloudConfig::new("sk-...")
    .with_mode(SpiderCloudMode::Smart);

let mut website = Website::new("https://example.com")
    .with_spider_cloud_config(cloud)
    .build()?;
```

`Smart` mode uses the unblocker only on pages that need it, so you don't pay for bypass on requests that work fine on their own.

> Free tier on signup at [spider.cloud](https://spider.cloud) — no card required.

## Install

| You want… | Run |
|---|---|
| Rust library | `cargo add spider` |
| Command-line tool | `cargo install spider_cli` |
| Node.js package | `npm i @spider-rs/spider-rs` |
| Python package | `pip install spider_rs` |
| MCP server (Claude, Cursor, …) | `cargo install spider_mcp` |
| Managed crawling | [spider.cloud](https://spider.cloud) |

## A little configuration

```rust
let mut website = Website::new("https://example.com")
    .with_limit(50)                    // concurrent requests
    .with_depth(10)                    // how deep to follow links
    .with_delay(500)                   // polite pause between hits (ms)
    .with_respect_robots_txt(true)
    .with_subdomains(true)
    .with_user_agent(Some("MyBot/1.0"))
    .with_stealth(true)
    .build()
    .unwrap();
```

Defaults are reasonable — you only set what you care about. Full reference in the [`Configuration` docs](https://docs.rs/spider/latest/spider/configuration/struct.Configuration.html).

Need JavaScript rendering? Turn on `features = ["chrome"]` and call `crawl_smart()` — Spider tries HTTP first and only spins up Chrome on pages that need it.

## A few things people do with it

- Ingest the open web into vector stores for LLM pipelines
- Watch sites for SEO or price changes
- Export pages as Markdown, JSON, or WARC archives
- Drive headless Chrome for AI browsing agents
- Build small search indexes on a single machine

→ 50+ runnable [examples](./examples/) to start from.

## Going deeper

- 📚 [Guides](https://spider.cloud/guides) — recipes and integrations
- 📖 [API docs](https://docs.rs/spider) — every option and method
- 💬 [Discord](https://discord.spider.cloud) — questions and ideas
- 🐛 [Issues](https://github.com/spider-rs/spider/issues) — bugs and feature requests

## Contributing

PRs welcome. See [`CONTRIBUTING.md`](./CONTRIBUTING.md).

```bash
cargo test -p spider                  # unit tests
RUN_LIVE_TESTS=1 cargo test           # live network tests
```

## License

[MIT](./LICENSE).
