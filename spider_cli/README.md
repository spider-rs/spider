# Spider CLI

![crate version](https://img.shields.io/crates/v/spider.svg)

Is a command line tool to utilize the spider.

## Dependencies

On Debian or other DEB based distributions:

```bash
$ sudo apt install openssl libssl-dev
```

On Fedora and other RPM based distributions:

```bash
$ sudo dnf install openssl-devel
```

## Usage

The cli is a binary so do not add it to your cargo.toml file.

```sh
cargo install spider_cli
```

## Cli

The following can also be ran via command line to run the crawler.
All website options are available except `website.on_link_find_callback`.

```sh
spider --domain https://choosealicense.com --verbose true --delay 2000 --user_agent something@1.20
```
