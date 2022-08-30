# Spider CLI

![crate version](https://img.shields.io/crates/v/spider.svg)

A fast command line spider or crawler.

## Dependencies

On Linux

- OpenSSL 1.0.1, 1.0.2, 1.1.0, or 1.1.1

## Usage

The CLI is a binary so do not add it to your `Cargo.toml` file.

```sh
cargo install spider_cli
```

## Cli

The following can also be ran via command line to run the crawler.
If you need loging pass in the `-v` flag.

```sh
spider -v --domain https://choosealicense.com crawl
```

Crawl and output all links visited to a file.

```sh
spider --domain https://choosealicense.com crawl -o > spider_choosealicense.json
```

```sh
spider_cli 1.13.0
madeindjs <contact@rousseau-alexandre.fr>, j-mendez <jeff@a11ywatch.com>
Multithreaded web crawler written in Rust.

USAGE:
    spider [OPTIONS] --domain <DOMAIN> [SUBCOMMAND]

OPTIONS:
    -b, --blacklist-url <BLACKLIST_URL>
            Comma seperated string list of pages to not crawl or regex with feature enabled

    -c, --concurrency <CONCURRENCY>
            How many request can be run simultaneously

    -d, --domain <DOMAIN>
            Domain to crawl

    -D, --delay <DELAY>
            Polite crawling delay in milli seconds

    -h, --help
            Print help information

    -r, --respect-robots-txt
            Respect robots.txt file

    -u, --user-agent <USER_AGENT>
            User-Agent

    -v, --verbose
            Print page visited on standard output

    -V, --version
            Print version information

SUBCOMMANDS:
    crawl     crawl the website extracting links
    help      Print this message or the help of the given subcommand(s)
    scrape    scrape the website extracting html and links
```

All features are available except the Website struct `on_link_find_callback` configuration option.
