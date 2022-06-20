# Spider

![crate version](https://img.shields.io/crates/v/spider.svg)

Multithreaded web crawler/indexer written in Rust main repo.

## Dependencies

On Linux

- OpenSSL 1.0.1, 1.0.2, 1.1.0, or 1.1.1

## Example

This is a basic blocking example crawling a web page, add spider to your `Cargo.toml`:

```toml
[dependencies]
spider = "1.8.3"
```

And then the code:

```rust,no_run
extern crate spider;

use spider::website::Website;

fn main() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.crawl();

    for page in website.get_pages() {
        println!("- {}", page.get_url());
    }
}
```

You can use `Configuration` object to configure your crawler:

```rust
// ..
let mut website: Website = Website::new("https://choosealicense.com");
website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
website.configuration.respect_robots_txt = true;
website.configuration.delay = 2000; // Defaults to 250 ms
website.configuration.concurrency = 10; // Defaults to number of cpus available * 4
website.configuration.user_agent = "myapp/version".to_string(); // Defaults to spider/x.y.z, where x.y.z is the library version
website.on_link_find_callback = |s| { println!("link target: {}", s); s }; // Callback to run on each link find

website.crawl();
```

## Regex Blacklisting

There is an optional "regex" crate that can be enabled:

```toml
[dependencies]
spider = { version = "1.8.3", features = ["regex"] }
```

```rust,no_run
extern crate spider;

use spider::website::Website;

fn main() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.configuration.blacklist_url.push("/licenses/".to_string());
    website.crawl();

    for page in website.get_pages() {
        println!("- {}", page.get_url());
    }
}
```
