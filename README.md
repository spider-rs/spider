# Spider

[![Crates.io](https://img.shields.io/crates/v/spider.svg)](https://crates.io/crates/spider)
[![Downloads](https://img.shields.io/crates/d/spider.svg)](https://crates.io/crates/spider)
[![Documentation](https://docs.rs/spider/badge.svg)](https://docs.rs/spider)

[Website](https://spider.cloud) |
[Guides](https://spider.cloud/guides) |
[API Docs](https://docs.rs/spider/latest/spider) |
[Examples](./examples/) |
[Discord](https://discord.spider.cloud)

The fastest web crawler and scraper for Rust. Get started at [spider.cloud](https://spider.cloud).

```toml
[dependencies]
spider = { version = "2", features = ["spider_cloud"] }
```

```rust
use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    // Get your API key at https://spider.cloud
    let mut website = Website::new("https://example.com")
        .with_spider_cloud("YOUR_API_KEY")
        // .with_spider_cloud_mode(SpiderCloudMode::Smart)    // proxy + auto-fallback on bot detection
        // .with_spider_cloud_mode(SpiderCloudMode::Fallback)  // direct first, API on failure
        .build()
        .unwrap();

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

Also supports [headless Chrome](./examples/chrome.rs), [WebDriver](./examples/webdriver.rs), and [AI automation](./spider_agent/).

## Get Spider

| Package | Install |
|---------|---------|
| [spider](https://crates.io/crates/spider) | `cargo add spider` |
| [spider_cli](./spider_cli/) | `cargo install spider_cli` |
| [spider-nodejs](https://github.com/spider-rs/spider-nodejs) | `npm i @spider-rs/spider-rs` |
| [spider-py](https://github.com/spider-rs/spider-py) | `pip install spider_rs` |
| [Spider Cloud](https://spider.cloud) | Managed crawling — free credits on signup |

## License

[MIT](LICENSE)
