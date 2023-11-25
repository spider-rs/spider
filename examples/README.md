# Examples

First `git clone https://github.com/spider-rs/spider.git` and `cd spider`.

## Basic

Simple concurrent crawl [Simple](./example.rs).

- `cargo run --example example`

Subscribe to realtime changes [Subscribe](./subscribe.rs).

- `cargo run --example subscribe`

Live handle index mutation example [Callback](./callback.rs).

- `cargo run --example callback`

Enable log output [Debug](./debug.rs).

- `cargo run --example debug`

Scrape the webpage with and gather html [Scrape](./scrape.rs).

- `cargo run --example scrape`

Scrape and download the html file to fs [Download HTML](./download.rs). \*Note: Enable feature flag [full_resources] to gather all files like css, jss, and etc.

- `cargo run --example download`

Scrape and download html to react components and store to fs [Download to React Component](./download.rs).

- `cargo run --example download_to_react`

Crawl the page and output the links via [Serde](./serde.rs).

- `cargo run --example serde --features serde`

Crawl links with a budget of amount of pages allowed [Budget](./budget.rs).

- `cargo run --example budget`

Crawl links at a given cron time [Cron](./cron.rs).

- `cargo run --example cron`