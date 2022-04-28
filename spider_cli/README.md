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
Website args are optional except `domain`. If you need verbose output pass in the `-v` flag.

```sh
spider -v --domain https://choosealicense.com crawl
```

Crawl and output all links visited on finished to a file.

```sh
spider  --domain https://choosealicense.com crawl -o > spider_choosealicense.json
```

All website options are available except `website.on_link_find_callback`.
