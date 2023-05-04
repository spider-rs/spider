# Spider Worker

![crate version](https://img.shields.io/crates/v/spider.svg)

A spider worker to decentralize the crawl lifting.

## Dependencies

This project depends on the [spider](../spider/) crate.

## Usage

The worker starts on port 3030 and the scraper for html gathering on 3031 by default. 

`SPIDER_WORKER_PORT=3030 SPIDER_WORKER_SCRAPER_PORT=3031 cargo run`

## Feature Flags

1. `scrape` - When the html is needed run the instance with the flag. Requires spider feature flag matching on the client to start. This also starts the instance on port 3031 instead.
1. `all` - Start the basic worker to gather links and scraper together.

## Ports

By default the instance runs on port `3030`.
The scraper runs on port `3031` when enabled.

## Todo

1. Allow port configuration.