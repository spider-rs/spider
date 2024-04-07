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
spider = "1.89.16"
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
website.configuration.budget = Some(spider::hashbrown::HashMap::from([(spider::CaseInsensitiveString::new("*"), 300), (spider::CaseInsensitiveString::new("/licenses"), 10)])); // Defaults to None - Requires the `budget` feature flag
website.configuration.cron_str = "1/5 * * * * *".into(); // Defaults to empty string - Requires the `cron` feature flag
website.configuration.cron_type = spider::website::CronType::Crawl; // Defaults to CronType::Crawl - Requires the `cron` feature flag
website.configuration.limit = 300; // The limit of pages crawled. By default there is no limit.
website.configuration.cache = false; // HTTP caching. Requires the `cache` or `chrome` feature flag.

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
   .with_budget(Some(spider::hashbrown::HashMap::from([("*", 300), ("/licenses", 10)])))
   .with_limit(300)
   .with_caching(false)
   .with_external_domains(Some(Vec::from(["https://creativecommons.org/licenses/by/3.0/"].map( |d| d.to_string())).into_iter()))
   .with_headers(None)
   .with_blacklist_url(Some(Vec::from(["https://choosealicense.com/licenses/".into()])))
   .with_cron("1/5 * * * * *", Default::Default())
   .with_proxies(None);
```

## Features

We have the following optional feature flags.

```toml
[dependencies]
spider = { version = "1.89.16", features = ["regex", "ua_generator"] }
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
1. `cache`: Enables HTTP caching request to disk.
1. `cache_mem`: Enables HTTP caching request to persist in memory.
1. `chrome`: Enables chrome headless rendering, use the env var `CHROME_URL` to connect remotely.
1. `chrome_store_page`: Store the page object to perform other actions. The page may be closed.
1. `chrome_screenshot`: Enables storing a screenshot of each page on crawl. Defaults the screenshots to the ./storage/ directory. Use the env variable `SCREENSHOT_DIRECTORY` to adjust the directory. To save the background set the env var `SCREENSHOT_OMIT_BACKGROUND` to false.
1. `chrome_headed`: Enables chrome rendering headful rendering.
1. `chrome_headless_new`: Use headless=new to launch the browser.
1. `chrome_cpu`: Disable gpu usage for chrome browser.
1. `chrome_stealth`: Enables stealth mode to make it harder to be detected as a bot.
1. `chrome_intercept`: Allows intercepting network request to speed up processing.
1. `cookies`: Enables cookies storing and setting to use for request.
1. `real_browser`: Enables the ability to bypass protected pages.
1. `cron`: Enables the ability to start cron jobs for the website.
1. `openai`: Enables OpenAI to generate dynamic browser executable scripts. Make sure to use the env var `OPENAI_API_KEY`.
1. `smart`: Enables smart mode. This runs request as HTTP until JavaScript rendering is needed. This avoids sending multiple network request by re-using the content.
1. `encoding`: Enables handling the content with different encodings like Shift_JIS.
1. `headers`: Enables the extraction of header information on each retrieved page. Adds a `headers` field to the page struct.
1. `decentralized_headers`: Enables the extraction of suppressed header information of the decentralized processing of IO.
This is needed if `headers` is set in both [spider](../spider/README.md) and [spider_worker](../spider_worker/README.md).

### Decentralization

Move processing to a worker, drastically increases performance even if worker is on the same machine due to efficient runtime split IO work.

```toml
[dependencies]
spider = { version = "1.89.16", features = ["decentralized"] }
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

### Handling headers with decentralisation

Without decentralisation the values of the headers for a page are unmodified.
When working with decentralized workers, each worker stores the headers retrieved
for the original request with prefixed element names (`"zz-spider-r--"`).

Using the feature `decentralized_headers` provides some useful tools to clean and extract the original header
entries under `spider::features::decentralized_headers`.

[WORKER_SUPPRESSED_HEADER_PREFIX]

### Subscribe to changes

Use the subscribe method to get a broadcast channel.

```toml
[dependencies]
spider = { version = "1.89.16", features = ["sync"] }
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
spider = { version = "1.89.16", features = ["regex"] }
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
spider = { version = "1.89.16", features = ["control"] }
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
spider = { version = "1.89.16", features = ["sync", "cron"] }
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

Connecting to Chrome can be done using the ENV variable `CHROME_URL`, if no connection is found a new browser is launched on the system. You do not need a chrome installation if you are connecting remotely. If you are not scraping content for downloading use
the feature flag [`chrome_intercept`] to possibly speed up request using Network Interception.

```toml
[dependencies]
spider = { version = "1.89.16", features = ["chrome", "chrome_intercept"] }
```

You can use `website.crawl_concurrent_raw` to perform a crawl without chromium when needed. Use the feature flag `chrome_headed` to enable headful browser usage if needed to debug.

```rust
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr")
        .with_chrome_intercept(cfg!(feature = "chrome_intercept"), true)
        .build()
        .unwrap();

    website.crawl().await;

    println!("Links found {:?}", website.get_links().len());
}
```

### Caching

Enabling HTTP cache can be done with the feature flag [`cache`] or [`cache_mem`].

```toml
[dependencies]
spider = { version = "1.89.16", features = ["cache"] }
```

You need to set `website.cache` to true to enable as well.

```rust
extern crate spider;

use spider::tokio;
use spider::website::Website;

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://rsseau.fr")
        .with_caching(true)
        .build()
        .unwrap();

    website.crawl().await;

    println!("Links found {:?}", website.get_links().len());
    /// next run to website.crawl().await; will be faster since content is stored on disk.
}
```

### Smart Mode

Intelligently run crawls using HTTP and JavaScript Rendering when needed. The best of both worlds to maintain speed and extract every page. This requires a chrome connection or browser installed on the system.

```toml
[dependencies]
spider = { version = "1.89.16", features = ["smart"] }
```

```rust,no_run
extern crate spider;

use spider::website::Website;
use spider::tokio;

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://choosealicense.com");
    website.crawl_smart().await;

    for link in website.get_links() {
        println!("- {:?}", link.as_ref());
    }
}
```

### OpenAI

Use OpenAI to generate dynamic scripts to drive the browser done with the feature flag [`openai`].

```toml
[dependencies]
spider = { version = "1.89.16", features = ["openai"] }
```

```rust
extern crate spider;

use spider::{tokio, website::Website, configuration::GPTConfigs};

#[tokio::main]
async fn main() {
    let mut website: Website = Website::new("https://google.com")
        .with_openai(Some(GPTConfigs::new("gpt-4-1106-preview", "Search for Movies", 256)))
        .with_limit(1)
        .build()
        .unwrap();

    website.crawl().await;
}
```

### Depth

Set a depth limit to prevent forwarding.

```toml
[dependencies]
spider = { version = "1.89.16", features = ["budget"] }
```

```rust,no_run
extern crate spider;

use spider::{tokio, website::Website};

#[tokio::main]
async fn main() {
    let mut website = Website::new("https://choosealicense.com").with_depth(3).build().unwrap();
    website.crawl().await;

    for link in website.get_links() {
        println!("- {:?}", link.as_ref());
    }
}
```

### Reusable Configuration

It is possible to re-use the same configuration for a crawl list.

```rust
extern crate spider;

use spider::configuration::Configuration;
use spider::{tokio, website::Website};
use std::io::Error;
use std::time::Instant;

const CAPACITY: usize = 5;
const CRAWL_LIST: [&str; CAPACITY] = [
    "https://rsseau.fr",
    "https://choosealicense.com",
    "https://jeffmendez.com",
    "https://spider-rs.github.io/spider-nodejs/",
    "https://spider-rs.github.io/spider-py/",
];

#[tokio::main]
async fn main() -> Result<(), Error> {
    let config = Configuration::new()
        .with_user_agent(Some("SpiderBot"))
        .with_blacklist_url(Some(Vec::from(["https://rsseau.fr/resume".into()])))
        .with_subdomains(false)
        .with_tld(false)
        .with_redirect_limit(3)
        .with_respect_robots_txt(true)
        .with_external_domains(Some(
            Vec::from(["http://loto.rsseau.fr/"].map(|d| d.to_string())).into_iter(),
        ))
        .build();

    let mut handles = Vec::with_capacity(CAPACITY);

    for website_url in CRAWL_LIST {
        match Website::new(website_url)
            .with_config(config.to_owned())
            .build()
        {
            Ok(mut website) => {
                let handle = tokio::spawn(async move {
                    println!("Starting Crawl - {:?}", website.get_domain().inner());

                    let start = Instant::now();
                    website.crawl().await;
                    let duration = start.elapsed();

                    let links = website.get_links();

                    for link in links {
                        println!("- {:?}", link.as_ref());
                    }

                    println!(
                        "{:?} - Time elapsed in website.crawl() is: {:?} for total pages: {:?}",
                        website.get_domain().inner(),
                        duration,
                        links.len()
                    );
                });

                handles.push(handle);
            }
            Err(e) => println!("{:?}", e)
        }
    }

    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}

```

### Blocking

If you need a blocking sync implementation use a version prior to `v1.12.0`.
