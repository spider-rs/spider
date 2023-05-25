# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
  - [crawl-speed](#crawl-speed)

## Benchmark Results

All test are done remotely using 10-100 samples with one decentralized worker on the same host machine. We want to keep the benchmarks close to a real world scenario reason for network IO instead of spinning up a local website.

### mac

```sh
----------------------
mac Apple M1 Max
10-core CPU
64 GB of RAM memory
1 TB of SSD disk space
-----------------------

Test url: `https://rsseau.fr`

185 pages
```

|                                          | `libraries`           |
| :--------------------------------------- | :-------------------- |
| **`Rust[spider]: crawl 10 samples`**     | `73ms` (✅ **1.00x**) |
| **`Go[crolly]: crawl 10 samples`**       | `32s`  (✅ **1.00x**) |
| **`Node.js[crawler]: crawl 10 samples`** | `15s`  (✅ **1.00x**) |
| **`C[wget]: crawl 10 samples`**          | `70s`  (✅ **1.00x**) |

### linux

```sh
----------------------
linux ubuntu-latest
2-core CPU
7 GB of RAM memory
14 GB of SSD disk space
-----------------------

Test url: `https://rsseau.fr`

185 pages
```

|                                          | `libraries`           |
| :--------------------------------------- | :-------------------- |
| **`Rust[spider]: crawl 10 samples`**     | `50ms` (✅ **1.00x**) |
| **`Go[crolly]: crawl 10 samples`**       | `30s`  (✅ **1.00x**) |
| **`Node.js[crawler]: crawl 10 samples`** | `3.4s` (✅ **1.00x**) |
| **`C[wget]: crawl 10 samples`**          | `60s`  (✅ **1.00x**) |

The concurrent benchmarks are averaged across 10 individual runs for 10 concurrent crawls with 10 sample counts.

In order for us to get better metrics we need to test the concurrency and simultaneous runs with a larger website. Favorably a website that can spin up inside the local container to avoid latency issues. The multi-threaded crawling capabilities shines brighter the larger the website.
Currently even with a small website this package still runs faster than the top OSS crawlers to date. [Spider](https://github.com/spider-rs/spider/tree/main/spider) is capable of crawling over 10k pages between 1-10 minutes depending on the website and OS. When spider is used decentralized it can handle IO within fractions of the time depending on the specs and amount of workers. The IO handling in linux performs drastically better than macOS and windows.

_Note_: Nodejs concurrency heavily impacts each additional run. As soon as you add multiple crawlers with nodejs the performance reduces over 2x plus per, while other lanaguages that can handle concurrency scale effectively.

## CI

You need a dedicated machine to get non flakey results. [Github Actions](https://github.com/spider-rs/spider/actions) results may differ across runs due to the shared env and the crawler built to scale across workloads.