# Spider

![crate version](https://img.shields.io/crates/v/spider.svg)

Multithreaded Web spider crawler written in Rust.

## Depensencies

~~~bash
$ apt install openssl libssl-dev
~~~

## Usage

Add this dependency to your _Cargo.toml_ file.

~~~toml
[dependencies]
spider = "1.0.2"
~~~

and then you'll be able to use library. Here a simple example

~~~rust
extern crate spider;

use spider::website::Website;

fn main() {
    let mut website: Website = Website::new("https://choosealicense.com");
    website.crawl();

    for page in website.get_pages() {
        println!("- {}", page.get_url());
    }
}
~~~

You can use `Configuration` object to configure your crawler:

~~~rust
// ..
let mut website: Website = Website::new("https://choosealicense.com");
website.configuration.blacklist_url.push("https://choosealicense.com/licenses/".to_string());
website.configuration.respect_robots_txt = true;
website.configuration.verbose = true;
website.crawl();
// ..
~~~

## TODO

- [x] multi-threaded system
- [x] respect _robot.txt_ file
- [x] add configuration object for polite delay, etc..
- [ ] add polite delay
- [ ] parse command line arguments

## Contribute

I am open-minded to any contribution. Just fork & `commit` on another branch.


