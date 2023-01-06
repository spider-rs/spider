# Examples

First `git clone https://github.com/spider-rs/spider.git` and `cd spider`.

## Basic

Simple concurrent crawl [Simple](./example.rs).

- `cargo run --example example`

Live handle index example [Callback](./callback.rs).

- `cargo run --example callback`

Enable log output [Debug](./debug.rs).

- `cargo run --example debug`

Scrape the webpage with and gather html [Scrape](./scrape.rs).

- `cargo run --example scrape`

Scrape and download the html file to fs [Download HTML](./download.rs). \*Note: Only HTML is downloaded.

- `cargo run --example download`

Scrape and download html to react components and store to fs [Download to React Component](./download.rs).

- `cargo run --example download_to_react`
