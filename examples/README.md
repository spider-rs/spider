# Examples

First `git clone https://github.com/spider-rs/spider.git` and `cd spider`. Use the release flag for the best performance `--release` when running the examples below.

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

Crawl links with chrome headed rendering [Chrome](./chrome.rs).

- `cargo run --example chrome --features chrome_headed`

Crawl links with view port configuration [Chrome Viewport](./chrome_viewport.rs).

- `cargo run --example chrome_viewport --features chrome`

Take a screenshot of a page during crawl [Chrome Screenshot](./chrome_screenshot.rs).

- `cargo run --example chrome_screenshot --features="spider/sync spider/chrome spider/chrome_store_page"`

Crawl links with smart mode detection. Runs HTTP by default until Chrome Rendering is needed. [Smart](./smart.rs).

- `cargo run --example smart --features smart`

Use different encodings for the page. [Encoding](./encoding.rs).

- `cargo run --example encoding --features encoding`

Use advanced configuration re-use. [Advanced Configuration](./advanced_configuration.rs).

- `cargo run --example advanced_configuration`

Use URL globbing for a domain. [URL Globbing](./url_glob.rs).

- `cargo run --example glob --features glob`

Use URL globbing for a domain and subdomains. [URL Globbing Subdomains](./url_glob_subdomains.rs).

- `cargo run --example url_glob_subdomains --features glob`

Downloading files in a subscription. [Subscribe Download](./subscribe_download.rs).

- `cargo run --example subscribe_download`

Add links to gather mid crawl. [Queue](./queue.rs).

- `cargo run --example queue`

Use OpenAI to get custom Javascript to run in a browser. [OpenAI](./openai.rs). Make sure to set OPENAI_API_KEY=$MY_KEY as an env variable or pass it in before the script.

- `cargo run --example openai`

or 

- `OPENAI_API_KEY=replace_me_with_key cargo run --example openai`

or setting multiple actions to drive the browser

- `OPENAI_API_KEY=replace_me_with_key cargo run --example openai_multi`

or to get custom data from the GPT with JS scripts if needed.

- `OPENAI_API_KEY=replace_me_with_key cargo run --example openai_extra`