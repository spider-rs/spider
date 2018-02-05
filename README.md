# Spider

![crate version](https://img.shields.io/crates/v/spider.svg)

Web spider framework that can spider a domain and collect pages it visits.

## Depensencies

~~~bash
$ apt install openssl libssl-dev
~~~

## Usage

## from source for command line usages

~~~bash
$ git clone https://github.com/madeindjs/spider.git
$ cd spider
$ cargo build --release
$ ./target/debug/rust-crawler http://localhost:4000
~~~

Will produce something like this

    - http://localhost:4000/
    - http://localhost:4000/portfolio
    - http://localhost:4000/resume
    - http://localhost:4000/blog

## as crate for librairy usage

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

- [ ]: multi-threaded system
- [ ]: respect _robot.txt_ file
- [ ]: add configuratioon object for polite delay, etc..
- [ ]: parse command line arguments

## Contribute

I am open-minded to any contribution. Just fork & `commit` on another branch.


