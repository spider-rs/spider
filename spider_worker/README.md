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
1. `full_resources` - Start the basic worker to gather links and scraper together.
1. `tls` - Enable tls support use the env variables `SPIDER_WORKER_CERT_PATH` for the `.pem` file and `SPIDER_WORKER_KEY_PATH` with your `.rsa` file. Defaults to `/cert.pem` and `/key.rsa`.

## Ports

By default the instance runs on port `3030` use `SPIDER_WORKER_PORT` to adjust the port.
The scraper runs on port `3031` when enabled use `SPIDER_WORKER_SCRAPER_PORT` to adjust the port.