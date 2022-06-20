# benches

This folder consists of benches between different cases within the library and including comparison between other choices.

## Initial benchmarks

We have comparisons set against 3 different languages and libs that can be used to crawl a web page.

### Crawl

How fast can we crawl all pages on a medium sized website. Tests are ordered between the largest to smallest runtimes needed. All examples use the same html selector to gather the pages for a website.

10 samples between each run on `https://rsseau.fr`:

1. [Node.js](./node_crawler.rs) - node-crawler
1. [Go](./go_crolly.rs) - Crolly
1. [Rust](./crawl.rs) - Spider

You can view the latest [benches here](./BENCHMARKS.md)
