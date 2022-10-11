# Spider

![crate version](https://img.shields.io/crates/v/spider.svg)

Multithreaded web crawler/indexer using [isolates](https://research.cs.wisc.edu/areas/os/Seminar/schedules/papers/Deconstructing_Process_Isolation_final.pdf) and IPC channels for communication.

## Dependencies

On Linux

- OpenSSL 1.0.1, 1.0.2, 1.1.0, or 1.1.1

## Example

This is a basic blocking example crawling a web page, add spider to your `Cargo.toml`:

```toml
[dependencies]
spider = "1.16.4"
```

And then the code:

```rust,no_run
extern crate spider;

use spider::website::Website;
use spider::tokio;

#[tokio::main]
async fn main() {
    let url = "https://choosealicense.com";
    let mut website: Website = Website::new(&url);
    website.crawl().await;

    for page in website.get_pages() {
        println!("- {}", page.get_url());
    }

    // optional: reset the crawl base or another domain for a new crawl
    website.reset(&url);
    // re-crawl the domain
    website.crawl().await;
}
```

You can use `Configuration` object to configure your crawler:

```rust
// ..
let mut website: Website = Website::new("https://choosealicense.com");
website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
website.configuration.respect_robots_txt = true;
website.configuration.subdomains = true;
website.configuration.tld = false;
website.configuration.delay = 2000; // Defaults to 250 ms
website.configuration.channel_buffer = 100; // Defaults to 50 - tune this depending on on_link_find_callback
website.configuration.user_agent = "myapp/version".to_string(); // Defaults to spider/x.y.z, where x.y.z is the library version
website.on_link_find_callback = |s| { println!("link target: {}", s); s }; // Callback to run on each link find

website.crawl().await;
```

## Regex Blacklisting

There is an optional "regex" crate that can be enabled:

```toml
[dependencies]
spider = { version = "1.16.4", features = ["regex"] }
```

```rust,no_run
extern crate spider;

use spider::website::Website;
use spider::tokio;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.blacklist_url.push("/licenses/".to_string());
    website.crawl().await;

    for page in website.get_pages() {
        println!("- {}", page.get_url());
    }
}
```

## Features

Currently we have three optional feature flags. Regex blacklisting and randomizing User-Agents.

```toml
[dependencies]
spider = { version = "1.16.4", features = ["regex", "ua_generator"] }
```

[Jemalloc](https://github.com/jemalloc/jemalloc) performs better for concurrency and allows memory to release easier.

This changes the global allocator of the program so test accordingly to measure impact.

```toml
[dependencies]
spider = { version = "1.16.4", features = ["jemalloc"] }
```

## Blocking

If you need a blocking sync imp use a version prior to `v1.12.0`.
