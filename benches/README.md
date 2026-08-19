# benches

[![Benches](https://github.com/madeindjs/spider/actions/workflows/bench.yml/badge.svg)](https://github.com/madeindjs/spider/actions/workflows/bench.yml)

This folder holds benches comparing cases within the library, and comparing the library against other crawlers.

## Initial benchmarks

We have comparisons set against 4 different languages and libs that can be used to crawl a web page.

### Crawl

How fast can we crawl all pages on a medium sized website. Tests are ordered between the largest to smallest runtimes needed. All examples use the same html selector to gather the pages for a website.

10 samples between each run on `https://spider.cloud`:

1. [Node.js](./node_crawler.rs) - node-crawler
1. [Go](./go_colly.rs) - Colly
1. [Rust](./crawl.rs) - Spider
1. C - wget

## Notes

1. nodejs takes the cpu to 100% when crawling and performance suffers when concurrent.
1. wget under performs once you account for latency.

You can view the latest [benches here](./BENCHMARKS.md)
