# Spider

![crate version](https://img.shields.io/crates/v/spider.svg)

Multithreaded async crawler/indexer using [isolates](https://research.cs.wisc.edu/areas/os/Seminar/schedules/papers/Deconstructing_Process_Isolation_final.pdf) and IPC channels for communication with the ability to run decentralized.

## Dependencies

On Linux

- OpenSSL 1.0.1, 1.0.2, 1.1.0, or 1.1.1

## Example

This is a basic async example crawling a web page, add spider to your `Cargo.toml`:

```toml
[dependencies]
spider = "1.60.0"
```

And then the code:

```rust,no_run
extern crate spider;

use spider::website::Website;
use spider::tokio;

#[tokio::main]
async fn main() {
    let url = "https://choosealicense.com";
    let mut website = Website::new(&url);
    website.crawl().await;

    for link in website.get_links() {
        println!("- {:?}", link.as_ref());
    }
}
```

You can use `Configuration` object to configure your crawler:

```rust
// ..
let mut website = Website::new("https://choosealicense.com");

website.configuration.respect_robots_txt = true;
website.configuration.subdomains = true;
website.configuration.tld = false;
website.configuration.delay = 0; // Defaults to 0 ms due to concurrency handling
website.configuration.request_timeout = None; // Defaults to 15000 ms
website.configuration.http2_prior_knowledge = false; // Enable if you know the webserver supports http2
website.configuration.user_agent = Some("myapp/version".into()); // Defaults to using a random agent
website.on_link_find_callback = Some(|s, html| { println!("link target: {}", s); (s, html)}); // Callback to run on each link find - useful for mutating the url, ex: convert the top level domain from `.fr` to `.es`.
website.configuration.blacklist_url.get_or_insert(Default::default()).push("https://choosealicense.com/licenses/".into());
website.configuration.proxies.get_or_insert(Default::default()).push("socks5://10.1.1.1:12345".into()); // Defaults to None - proxy list.
website.budget = Some(spider::hashbrown::HashMap::from([(spider::CaseInsensitiveString::new("*"), 300), (spider::CaseInsensitiveString::new("/licenses"), 10)])); // Defaults to None - Requires the `budget` feature flag
website.cron_str = "1/5 * * * * *".into(); // Defaults to empty string - Requires the `cron` feature flag
website.cron_type = spider::website::CronType::Crawl; // Defaults to CronType::Crawl - Requires the `cron` feature flag

website.crawl().await;
```

The builder pattern is also available v1.33.0 and up: 

```rust
let mut website = Website::new("https://choosealicense.com");

website
    .with_respect_robots_txt(true)
    .with_subdomains(true)
    .with_tld(false)
    .with_delay(0)
    .with_request_timeout(None)
    .with_http2_prior_knowledge(false)
    .with_user_agent(Some("myapp/version".into()))
    // requires the `budget` feature flag
    .with_budget(Some(spider::hashbrown::HashMap::from([("*", 300), ("/licenses", 10)])))
    .with_external_domains(Some(Vec::from(["https://creativecommons.org/licenses/by/3.0/"].map(|d| d.to_string())).into_iter()))
    .with_headers(None)
    .with_blacklist_url(Some(Vec::from(["https://choosealicense.com/licenses/".into()])))
    // requires the `cron` feature flag
    .with_cron("1/5 * * * * *", Default::Default());
    .with_proxies(None);
```

## Features

We have a couple optional feature flags. Regex blacklisting, jemaloc backend, globbing, fs temp storage, decentralization, serde, gathering full assets, and randomizing user agents.

```toml
[dependencies]
spider = { version = "1.60.0", features = ["regex", "ua_generator"] }
```

1. `ua_generator`: Enables auto generating a random real User-Agent.
1. `regex`: Enables blacklisting paths with regx
1. `jemalloc`: Enables the [jemalloc](https://github.com/jemalloc/jemalloc) memory backend.
1. `decentralized`: Enables decentralized processing of IO, requires the [spider_worker](../spider_worker/README.md) startup before crawls.
1. `sync`: Subscribe to changes for Page data processing async. [Enabled by default]
1. `budget`: Allows setting a crawl budget per path with depth.
1. `control`: Enables the ability to pause, start, and shutdown crawls on demand.
1. `full_resources`: Enables gathering all content that relates to the domain like CSS, JS, and etc.
1. `serde`: Enables serde serialization support.
1. `socks`: Enables socks5 proxy support.
1. `glob`: Enables [url glob](https://everything.curl.dev/cmdline/globbing) support.
1. `fs`: Enables storing resources to disk for parsing (may greatly increases performance at the cost of temp storage).
1. `js`: Enables javascript parsing links created with the alpha [jsdom](https://github.com/a11ywatch/jsdom) crate.
1. `sitemap`: Include sitemap pages in results.
1. `time`: Enables duration tracking per page.
1. `chrome`: Enables chrome headless rendering, use the env var `CHROME_URL` to connect remotely.
1. `chrome_screenshot`: Enables storing a screenshot of each page on crawl. Defaults the screenshots to the ./storage/ directory. Use the env variable `SCREENSHOT_DIRECTORY` to adjust the directory. To save the background set the env var `SCREENSHOT_OMIT_BACKGROUND` to false.
1. `chrome_headed`: Enables chrome rendering headful rendering [experimental].
1. `chrome_cpu`: Disable gpu usage for chrome browser.
1. `chrome_stealth`: Enables stealth mode to make it harder to be detected as a bot.
1. `cookies`: Enables cookies storing and setting to use for request.
1. `cron`: Enables the ability to start cron jobs for the website.
1. `smart`: Enables smart mode. This runs request as HTTP until JavaScript rendering is needed. This avoids sending multiple network request by re-using the content.

### Decentralization

Move processing to a worker, drastically increases performance even if worker is on the same machine due to efficient runtime split IO work.

```toml
[dependencies]
spider = { version = "1.60.0", features = ["decentralized"] }
```

```sh
# install the worker
cargo install spider_worker
# start the worker [set the worker on another machine in prod]
RUST_LOG=info SPIDER_WORKER_PORT=3030 spider_worker
# start rust project as normal with SPIDER_WORKER env variable
SPIDER_WORKER=http://127.0.0.1:3030 cargo run --example example --features decentralized
```

The `SPIDER_WORKER` env variable takes a comma seperated list of urls to set the workers. If the `scrape` feature flag is enabled, use the `SPIDER_WORKER_SCRAPER` env variable to determine the scraper worker.

### Subscribe to changes

Use the subscribe method to get a broadcast channel.

```toml
[dependencies]
spider = { version = "1.60.0", features = ["sync"] }
```

```rust,no_run
extern crate spider;

use spider::website::Website;
use spider::tokio;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://choosealicense.com");
    let mut rx2 = website.subscribe(16).unwrap();

    let join_handle = tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            println!("{:?}", res.get_url());
        }
    });

    website.crawl().await;
}
```

### Regex Blacklisting

Allow regex for blacklisting routes

```toml
[dependencies]
spider = { version = "1.60.0", features = ["regex"] }
```

```rust,no_run
extern crate spider;

use spider::website::Website;
use spider::tokio;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://choosealicense.com");
    website.configuration.blacklist_url.push("/licenses/".into());
    website.crawl().await;

    for link in website.get_links() {
        println!("- {:?}", link.as_ref());
    }
}
```

### Pause, Resume, and Shutdown

If you are performing large workloads you may need to control the crawler by enabling the `control` feature flag:

```toml
[dependencies]
spider = { version = "1.60.0", features = ["control"] }
```

```rust
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    use spider::utils::{pause, resume, shutdown};
    let url = "https://choosealicense.com/";
    let mut website: Website = Website::new(&url);

    tokio::spawn(async move {
        pause(url).await;
        sleep(tokio::time::Duration::from_millis(5000)).await;
        resume(url).await;
        // perform shutdown if crawl takes longer than 15s
        sleep(tokio::time::Duration::from_millis(15000)).await;
        // you could also abort the task to shutdown crawls if using website.crawl in another thread.
        shutdown(url).await;
    });

    website.crawl().await;
}
```

### Scrape/Gather HTML

```rust
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    use std::io::{Write, stdout};

    let url = "https://choosealicense.com/";
    let mut website = Website::new(&url);

    website.scrape().await;

    let mut lock = stdout().lock();

    let separator = "-".repeat(url.len());

    for page in website.get_pages().unwrap().iter() {
        writeln!(
            lock,
            "{}\n{}\n\n{}\n\n{}",
            separator,
            page.get_url_final(),
            page.get_html(),
            separator
        )
        .unwrap();
    }
}
```

### Cron Jobs

Use cron jobs to run crawls continuously at anytime.

```toml
[dependencies]
spider = { version = "1.60.0", features = ["sync", "cron"] }
```

```rust,no_run
extern crate spider;

use spider::website::{Website, run_cron};
use spider::tokio;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://choosealicense.com");
    // set the cron to run or use the builder pattern `website.with_cron`.
    website.cron_str = "1/5 * * * * *".into();

    let mut rx2 = website.subscribe(16).unwrap();

    let join_handle = tokio::spawn(async move {
        while let Ok(res) = rx2.recv().await {
            println!("{:?}", res.get_url());
        }
    });

    // take ownership of the website. You can also use website.run_cron, except you need to perform abort manually on handles created.
    let mut runner = run_cron(website).await;

    println!("Starting the Runner for 10 seconds");
    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    let _ = tokio::join!(runner.stop(), join_handle);
}
```

### Chrome

```toml
[dependencies]
spider = { version = "1.60.0", features = ["chrome"] }
```

You can use `website.crawl_concurrent_raw` to perform a crawl without chromium when needed. Use the feature flag `chrome_headed` to enable headful browser usage if needed to debug.

### Blocking

If you need a blocking sync implementation use a version prior to `v1.12.0`.
