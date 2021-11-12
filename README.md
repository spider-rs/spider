# Spider

![crate version](https://img.shields.io/crates/v/spider.svg)

Multithreaded web crawler written in Rust.

## Dependencies

On Debian or other DEB based distributions:

```bash
$ sudo apt install openssl libssl-dev
```

On Fedora and other RPM based distributions:

```bash
$ sudo dnf install openssl-devel
```

## Usage

Add this dependency to your _Cargo.toml_ file.

```toml
[dependencies]
spider = "1.2.1"
```

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
website.configuration.verbose = true; // Defaults to false
website.configuration.delay = 2000; // Defaults to 250 ms
website.configuration.concurrency = 10; // Defaults to 4
website.configuration.user_agent = "myapp/version"; // Defaults to spider/x.y.z, where x.y.z is the library version
website.crawl();
```

You can get a working example at [`example.rs`](./example.rs) and run it with

```sh
cargo run --example example
```

## TODO

- [x] multi-threaded system
- [x] respect _robot.txt_ file
- [x] add configuration object for polite delay, etc..
- [x] add polite delay
- [ ] parse command line arguments

## Contribute

I am open-minded to any contribution. Just fork & `commit` on another branch.
