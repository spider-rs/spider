# benches

This folder consists of benches between different cases within the library and including comparison between other choices.

## Initial bench marks

We have comparisons set against 3 different languages and libs that can be used to crawl a web page.

## Setup

1. `npm install node-crawler`
1. `go mod init example.com/spider && go get github.com/gocolly/colly/v2`

## Crawl

How fast can we crawl all pages on a medium sized website. Tests are ordered between the largest to smallest runtimes needed. All examples use the same html selector to gather the pages for a website.

### v1.6.0

Case: `https://rsseau.fr`

10x simultaneous runs each.

1. `Node.js` - node-crawler
   . [example](./node_crawler.rs) recursive stack buffer.
1. `Go Lang` - Crolly
   . [example](./go_crolly.rs) recursive stack buffer.
1. `Rust` - Spider
   . [example](./crawl.rs) default example from CLI.
