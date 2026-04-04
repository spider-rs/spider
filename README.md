# Spider

[![Crates.io](https://img.shields.io/crates/v/spider.svg)](https://crates.io/crates/spider)
[![Downloads](https://img.shields.io/crates/d/spider.svg)](https://crates.io/crates/spider)
[![Documentation](https://docs.rs/spider/badge.svg)](https://docs.rs/spider)

[Website](https://spider.cloud) |
[Guides](https://spider.cloud/guides) |
[API](https://docs.rs/spider/latest/spider) |
[Examples](./examples/) |
[Discord](https://discord.spider.cloud)

The fastest web crawler and scraper for Rust.

## Quick Start

```toml
[dependencies]
spider = { version = "2", features = ["spider_cloud"] }
```

```rust
use spider::{
    configuration::{SpiderCloudConfig, SpiderCloudMode, SpiderCloudReturnFormat},
    tokio, // re-export
    website::Website,
};

#[tokio::main]
async fn main() {
    // Get your API key free at https://spider.cloud
    let config = SpiderCloudConfig::new("YOUR_API_KEY")
        .with_mode(SpiderCloudMode::Smart)
        .with_return_format(SpiderCloudReturnFormat::Markdown);

    let mut website = Website::new("https://example.com")
        .with_limit(10)
        .with_spider_cloud_config(config)
        .build()
        .unwrap();

    let mut rx = website.subscribe(16);

    tokio::spawn(async move {
        while let Ok(page) = rx.recv().await {
            let url = page.get_url();
            let markdown = page.get_content();
            let status = page.status_code;

            println!("[{status}] {url}\n---\n{markdown}\n");
        }
    });

    website.crawl().await;
    website.unsubscribe();
}
```

Also supports [headless Chrome](./examples/chrome.rs), [WebDriver](./examples/webdriver.rs), and [AI automation](./spider_agent/).

## Install

| Package | Command |
|---------|---------|
| [spider](https://crates.io/crates/spider) | `cargo add spider` |
| [spider_cli](./spider_cli/) | `cargo install spider_cli` |
| [spider-nodejs](https://github.com/spider-rs/spider-nodejs) | `npm i @spider-rs/spider-rs` |
| [spider-py](https://github.com/spider-rs/spider-py) | `pip install spider_rs` |
| [Spider Cloud](https://spider.cloud) | Managed crawling — free credits on signup |

## License

[MIT](LICENSE)
