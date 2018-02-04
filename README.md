# Spider

Web spider framework that can spider a domain and collect useful informations about the pages it visits.

## Depensencies

~~~bash
$ apt install openssl libssl-dev
~~~

## Instalation

    $ cargo build --release

Run either with:

`cargo run`

or directly with

`./target/debug/rust-crawler`

## Use

~~~rust
let mut localhost = Website::new("http://localhost:4000");
localhost.crawl();
~~~



