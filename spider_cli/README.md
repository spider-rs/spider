# Spider CLI

![crate version](https://img.shields.io/crates/v/spider.svg)

A fast command-line spider (web crawler) for high-performance website scraping.

## Dependencies

On Linux

- OpenSSL 1.0.1, 1.0.2, 1.1.0, or 1.1.1

Note: You need to have `pkg-config` installed otherwise `openssl` will not be recognized by cargo.

```bash
# On Ubuntu:
apt install pkg-config
```

## Usage

The CLI is a binary so do not add it to your `Cargo.toml` file.

```sh
# default install (includes chrome support)
cargo install spider_cli
# optional smart mode (HTTP first, browser fallback)
cargo install -F smart spider_cli
```

## Cli

Run crawls with explicit runtime mode control:

```sh
# HTTP mode (default)
spider --url https://choosealicense.com crawl --output-links
```

```sh
# Browser mode on demand
spider --url https://choosealicense.com --headless crawl --output-links
```

```sh
# Force HTTP-only even in chrome-enabled builds
spider --url https://choosealicense.com --http crawl --output-links
```

Crawl and output all links visited to a file.

```sh
spider --url https://choosealicense.com crawl -o > spider_choosealicense.json
```

Download all html to local destination. Use the option `-t` to pass in the target destination folder.

```sh
spider --url https://choosealicense.com download -t _temp_spider_downloads
```

Set a crawl budget and only crawl one domain.

```sh
spider --url https://choosealicense.com --budget "*,1" crawl -o
```

Set a crawl budget and only allow 10 pages matching the /blog/ path and limit all pages to 100.

```sh
spider --url https://choosealicense.com --budget "*,100,/blog/,10" crawl -o
```

Get all the resources for the page.

```sh
spider --url https://choosealicense.com --full-resources crawl -o
```

```sh
The fastest web crawler CLI written in Rust.

Usage: spider [OPTIONS] --url <URL> [COMMAND]

Commands:
  crawl     Crawl the website extracting links
  scrape    Scrape the website extracting html and links returning the output as jsonl
  download  Download html markup to destination
  help      Print this message or the help of the given subcommand(s)

Options:
  -u, --url <URL>
          The website URL to crawl
  -r, --respect-robots-txt
          Respect robots.txt file
  -s, --subdomains
          Allow sub-domain crawling
  -t, --tld
          Allow all tlds for domain
  -H, --return-headers
          Return the headers of the page.  Requires the `headers` flag enabled
  -v, --verbose
          Print page visited on standard output
  -D, --delay <DELAY>
          Polite crawling delay in milli seconds
      --limit <LIMIT>
          The max pages allowed to crawl
      --blacklist-url <BLACKLIST_URL>
          Comma seperated string list of pages to not crawl or regex with feature enabled
  -a, --agent <AGENT>
          User-Agent
  -B, --budget <BUDGET>
          Crawl Budget preventing extra paths from being crawled. Use commas to split the path followed by the limit ex: "*,1" - to only allow one page
  -E, --external-domains <EXTERNAL_DOMAINS>
          Set external domains to group with crawl
  -b, --block-images
          Block Images from rendering when using Chrome. Requires the `chrome_intercept` flag enabled
  -d, --depth <DEPTH>
          The crawl depth limits
      --accept-invalid-certs
          Dangerously accept invalid certficates
      --full-resources
          Gather all content that relates to the domain like css,jss, and etc
      --headless
          Use browser rendering mode (headless) for crawl/scrape/download. Requires the `chrome` feature
      --http
          Force HTTP-only mode (no browser rendering), even when built with `chrome`
  -p, --proxy-url <PROXY_URL>
          The proxy url to use
      --spider-cloud-key <SPIDER_CLOUD_KEY>
          Spider Cloud API key. Sign up at https://spider.cloud for an API key
      --spider-cloud-mode <SPIDER_CLOUD_MODE>
          Spider Cloud mode: proxy (default), api, unblocker, fallback, or smart [default: proxy]
      --wait-for-idle-network <WAIT_FOR_IDLE_NETWORK>
          Wait for network request to be idle within a time frame period (500ms no network connections) with an optional timeout in milliseconds
      --wait-for-idle-network0 <WAIT_FOR_IDLE_NETWORK0>
          Wait for network request with a max timeout (0 connections) with an optional timeout in milliseconds
      --wait-for-almost-idle-network0 <WAIT_FOR_ALMOST_IDLE_NETWORK0>
          Wait for network to be almost idle with a max timeout (max 2 connections) with an optional timeout in milliseconds
      --wait-for-idle-dom <WAIT_FOR_IDLE_DOM>
          Wait for idle dom mutations for target element (defaults to "body") with a 30s timeout
      --wait-for-selector <WAIT_FOR_SELECTOR>
          Wait for a specific CSS selector to appear with a 60s timeout
      --wait-for-delay <WAIT_FOR_DELAY>
          Wait for a fixed delay in milliseconds
  -h, --help
          Print help
  -V, --version
          Print version
```

All features are available except the Website struct `on_link_find_callback` configuration option.
