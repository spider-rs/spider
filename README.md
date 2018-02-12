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
    let mut localhost = Website::new("http://localhost:4000");
    localhost.crawl();

    for page in localhost.get_pages() {
        println!("- {}", page.get_url());
    }
}
~~~


## TODO

- [x] multi-threaded system
- [x] respect _robot.txt_ file
- [x] add configuration object for polite delay, etc..
- [ ] add polite delay
- [ ] parse command line arguments

## Contribute

I am open-minded to any contribution. Just fork & `commit` on another branch.


