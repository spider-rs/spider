# Spider CLI

![crate version](https://img.shields.io/crates/v/spider.svg)

Is a command line tool to utilize the spider.

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
Website args are optional except `domain`.

```sh
spider --domain https://choosealicense.com --delay 2000 --blacklist-url license,books --user-agent something@1.20 crawl
```

All website options are available except `website.on_link_find_callback`.
