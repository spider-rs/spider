<p align="center">
  <a href="https://spider.cloud" target="_blank">
    <img src="https://avatars.githubusercontent.com/u/112983871?s=400&u=e03cc05523f015dd1f2a5ab9e6158de8a30821c2&v=4" alt="Spider" width="140" height="140">
  </a>
</p>

<h1 align="center">Spider</h1>

<h4 align="center">
  <a href="https://spider.cloud">Website</a> |
  <a href="https://spider.cloud/guides">Guides</a> |
  <a href="https://spider.cloud">Spider Cloud</a> |
  <a href="https://docs.rs/spider">API Docs</a> |
  <a href="./examples/">Examples</a> |
  <a href="https://discord.spider.cloud">Discord</a>
</h4>

<p align="center">
  <a href="https://crates.io/crates/spider"><img src="https://img.shields.io/crates/v/spider.svg" alt="Crates.io"></a>
  <a href="https://crates.io/crates/spider"><img src="https://img.shields.io/crates/d/spider.svg" alt="Downloads"></a>
  <a href="https://docs.rs/spider"><img src="https://docs.rs/spider/badge.svg" alt="Documentation"></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-informational" alt="License"></a>
</p>

<p align="center">⚡ Fast, reliable, low-latency web crawling for Rust 🕸️</p>

- **Fast** — 100k+ pages in 1–10 minutes, benchmarked against the real internet.
- **Low-latency** — pages stream the moment they're fetched. No batching, no waiting.
- **Reliable** — battle-tested concurrency, automatic retries, proxy hedging, and graceful shutdown.
- **Scales with you** — one script to a fleet of workers, same API.

---

## A taste

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

That's the whole program. Pages stream as they arrive. The crawler stops when there's nothing left to fetch.

Want JavaScript rendering? Add `features = ["chrome"]` and call `crawl_smart()` instead — Spider will use HTTP first and only spin up Chrome for pages that need it.

---

## What you can build

- **Scrapers** that turn live websites into Markdown, plain text, JSON, or WARC archives.
- **Pipelines** that ingest pages into search indexes, vector stores, or databases as they're fetched — no temp directories, no full-crawl waits.
- **AI agents** that browse, click, fill forms, and reason about pages using OpenAI, Gemini, or any OpenAI-compatible API.
- **Monitors** that re-crawl on a cron schedule and emit only what changed.
- **Headless browser automation** at scale — locally with Chrome or WebDriver, or remotely via Spider Cloud's managed browser pool.

---

## Things Spider does well

**Streams in real time.** Subscribe once, get every page as it lands. Spider uses Tokio broadcast channels and event-driven wakeups under the hood — your consumer is never starved and the crawler never blocks on a slow downstream.

**Handles JavaScript when it has to.** "Smart mode" starts with plain HTTP and transparently upgrades to headless Chrome the moment it detects a page needs it. You don't pay the Chrome tax on pages that don't.

**Survives the modern web.** Real-browser fingerprint emulation, header spoofing, proxy rotation, request hedging across proxies, and stealth Chrome — built in. For sites that fight back hard, [Spider Cloud's Smart mode](https://spider.cloud) auto-detects Cloudflare, Akamai, Imperva, and friends and switches to its unblocker without a config change.

**Respects the rules when you want it to.** Robots.txt, sitemaps, per-path budgets, depth limits, allow/deny lists, glob and regex filters, cron schedules — all one method call away.

**Stays out of your way.** The whole crawler is one fluent builder. Defaults are good. There's nothing you *have* to configure to get started.

**Scales when you need it.** Adaptive concurrency, per-domain rate limiting, HTTP/2 multiplexing, `io_uring` on Linux, request coalescing, and an optional decentralized mode that splits work across worker processes over IPC.

---

## Configure as much (or as little) as you want

```rust
use spider::website::Website;
use std::{collections::HashMap, time::Duration};

let mut website = Website::new("https://example.com")
    .with_limit(50)                        // concurrent requests
    .with_depth(10)                        // how deep to follow links
    .with_delay(500)                       // polite pause between hits (ms)
    .with_request_timeout(Some(Duration::from_secs(30)))
    .with_respect_robots_txt(true)
    .with_subdomains(true)
    .with_user_agent(Some("MyBot/1.0"))
    .with_blacklist_url(Some(vec!["/admin".into()]))
    .with_whitelist_url(Some(vec!["/blog".into()]))
    .with_proxies(Some(vec!["http://proxy:8080".into()]))
    .with_budget(Some(HashMap::from([("/blog", 100), ("*", 1000)])))
    .with_caching(true)
    .with_stealth(true)
    .build()
    .unwrap();
```

Every option is documented in the [`Configuration` API reference](https://docs.rs/spider/latest/spider/configuration/struct.Configuration.html).

---

## Spider Cloud

If you don't want to run proxies, manage residential IPs, keep a Chrome pool warm, or chase Cloudflare update cycles — point Spider at [Spider Cloud](https://spider.cloud) and the same code runs against a managed crawling backend. Free tier on signup.

```rust
use spider::configuration::{SpiderCloudConfig, SpiderCloudMode};

let cloud = SpiderCloudConfig::new("sk-...")
    .with_mode(SpiderCloudMode::Smart);

let mut website = Website::new("https://example.com")
    .with_spider_cloud_config(cloud)
    .build()?;
```

Five cloud modes — `Proxy`, `Api`, `Unblocker`, `Fallback`, `Smart` — let you trade cost for resilience without changing your crawler code.

---

## Install

| Use Spider as… | Command |
|----------------|---------|
| A Rust library | `cargo add spider` |
| A command-line tool | `cargo install spider_cli` |
| A Node.js package | `npm i @spider-rs/spider-rs` |
| A Python package | `pip install spider_rs` |
| An MCP server (for Claude, Cursor, etc.) | `cargo install spider_mcp` |
| Managed crawling | Sign up at [spider.cloud](https://spider.cloud) |

---

## What's in the box

The workspace ships nine crates. Most users only need `spider` itself.

| Crate | What it's for |
|-------|---------------|
| [`spider`](./spider/) | The crawler. Start here. |
| [`spider_cli`](./spider_cli/) | A standalone command-line crawler. |
| [`spider_worker`](./spider_worker/) | A worker process for decentralized crawls. |
| [`spider_agent`](./spider_agent/) | An autonomous AI browsing agent (Chrome / WebDriver + LLMs). |
| [`spider_mcp`](./spider_mcp/) | A Model Context Protocol server so AI tools can call Spider directly. |
| [`spider_utils`](./spider_utils/), [`spider_agent_types`](./spider_agent_types/), [`spider_agent_html`](./spider_agent_html/) | Supporting libraries. |

There are **50+ runnable examples** in [`examples/`](./examples/) — Chrome automation, screenshots, OpenAI extraction, WARC export, cron scheduling, decentralized crawling, sitemap-only mode, anti-bot setups, and more.

---

## Documentation

- **API reference** — <https://docs.rs/spider>
- **Guides & recipes** — <https://spider.cloud/guides>
- **Examples** — [`./examples/`](./examples/)
- **Development notes** — [`CLAUDE.md`](./CLAUDE.md)

---

## Community

- 💬 [Discord](https://discord.spider.cloud) — questions, ideas, show-and-tell
- 🐛 [GitHub Issues](https://github.com/spider-rs/spider/issues) — bug reports and feature requests
- 🗞️ [Spider blog](https://spider.cloud/blog) — release notes and deep dives

---

## Contributing

Spider is open source and we love good pull requests. See [`CONTRIBUTING.md`](./CONTRIBUTING.md). For tests:

```bash
cargo test -p spider                  # unit tests
RUN_LIVE_TESTS=1 cargo test           # live network tests
```

---

## License

[MIT](./LICENSE) — use it for anything, commercial or otherwise.
