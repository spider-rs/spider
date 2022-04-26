# Spider

![crate version](https://img.shields.io/crates/v/spider.svg)

Multithreaded web crawler written in Rust main repo.

## Dependencies

On Linux

- OpenSSL 1.0.1, 1.0.2, 1.1.0, or 1.1.1

````

## Usage

Add this dependency to your _Cargo.toml_ file.

```toml
[dependencies]
spider = "1.7.6"
````

Then you'll be able to use library. Here is a simple example:

```rust
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
website.configuration.user_agent = "myapp/version"; // Defaults to spider/x.y.z, where x.y.z is the library version
website.on_link_find_callback = |s| { println!("link target: {}", s); s }; // Callback to run on each link find

website.crawl();
```
