# CHANGELOG

## Unreleased

## v1.19.30

1. perf(crawl): add join handle task management

## v1.19.26

1. perf(links): add fast pre serialized url anchor link extracting and reduced memory usage
1. perf(links): fix case sensitivity handling
1. perf(crawl): reduce memory usage on link gathering
1. chore(crawl): remove `Website.reset` method and improve crawl handling resource usage ( `reset` not needed now  )
1. chore(crawl): add heap usage of links visited
1. perf(crawl): massive scans capability to utilize more cpu
1. feat(timeout): add optional `configuration.request_timeout` duration
1. build(tokio): remove unused `net` feature
1. chore(docs): add missing scrape section

## v1.10.7

- perf(req): enable brotli
- chore(tls): add ALPN tls defaults
- chore(statics): add initial static media ignore
- chore(robots): add shared client handling across parsers
- feat(crawl): add subdomain and tld crawling

## v1.6.1

- perf(links): filter dup links after async batch
- chore(delay): fix crawl delay thread groups
- perf(page): slim channel page sending required props

## v1.5.3

- feat(regex): add optional regex black listing

## v1.5.0

- chore(bin): fix bin executable [#17](https://github.com/madeindjs/spider/pull/17/commits/b41e25fc507c6cd3ef251d2e25c97b936865e1a9)
- feat(cli): add cli separation binary [#17](https://github.com/madeindjs/spider/pull/17/commits/b41e25fc507c6cd3ef251d2e25c97b936865e1a9)
- feat(robots): add robots crawl delay respect and ua assign [#24](https://github.com/madeindjs/spider/pull/24)
- feat(async): add async page body gathering
- perf(latency): add connection re-use across request [#25](https://github.com/madeindjs/spider/pull/25)

## v1.4.0

- feat(cli): add cli ability ([#16](https://github.com/madeindjs/spider/pull/16) thanks to [@j-mendez](https://github.com/j-mendez))
- feat(concurrency): dynamic concurrent cpu defaults ([#15](https://github.com/madeindjs/spider/pull/15) thanks to [@j-mendez](https://github.com/j-mendez))
- docs: add a changelog

## v1.3.1

- fix(crawl): fix field type ([#14](https://github.com/madeindjs/spider/pull/14) thanks to [@j-mendez](https://github.com/j-mendez))

## v1.3.0

- feat(crawl): callback to run when link is found ([#13](https://github.com/madeindjs/spider/pull/13) thanks to [@j-mendez](https://github.com/j-mendez))

## v1.2.0

- Add User Agent configuration ([#5](https://github.com/madeindjs/spider/pull/5) thanks to [@Dragnucs](https://github.com/Dragnucs))
- Add polite delay ([#6](https://github.com/madeindjs/spider/pull/6) thanks to [@Dragnucs](https://github.com/Dragnucs) )

## v1.1.3

- Handle page get errors ([#4](https://github.com/madeindjs/spider/pull/4) thanks to [@Dragnucs](https://github.com/Dragnucs))
- Fix link resolution ([#3](https://github.com/madeindjs/spider/pull/3) thanks to [@Dragnucs](https://github.com/Dragnucs))
