# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
  - [crawl-speed](#crawl-speed)

## Benchmark Results

```sh
----------------------
linux ubuntu-latest
2-core CPU
7 GB of RAM memory
14 GB of SSD disk space
-----------------------

Test url: `https://rsseau.fr`

15 pages
```

### crawl-speed

runs with 10 samples:

|                                          | `libraries`               |
| :--------------------------------------- | :------------------------ |
| **`Rust[spider]: crawl 10 samples`**     | `1.8375 s` (✅ **1.00x**) |
| **`Go[crolly]: crawl 10 samples`**       | `2.9417 s` (✅ **1.00x**) |
| **`Node.js[crawler]: crawl 10 samples`** | `2.9992 s` (✅ **1.00x**) |
| **`C[wget]: crawl 10 samples`**          | `12.019 s` (✅ **1.00x**) |

### crawl-speed-concurrentx10

10 concurrent runs with 10 samples:

|                                          | `libraries`                |
| :--------------------------------------- | :------------------------- |
| **`Rust[spider]: crawl 10 samples`**     | `2.1670 s` (✅ **10.00x**) |
| **`Go[crolly]: crawl 10 samples`**       | `3.4310 s` (✅ **10.00x**) |
| **`Node.js[crawler]: crawl 10 samples`** | `22.174 s` (✅ **10.00x**) |
| **`C[wget]: crawl 10 samples`**          | `20.952 s` (✅ **10.00x**) |

The concurrent benchmarks are averaged across 10 individual runs for 10 concurrent crawls with 10 sample counts.

In order for us to get better metrics we need to test the concurrency and simultaneous runs with a larger website and favorably a website that can spin up inside the local container so latency is not being tracked. The multi-threaded crawling capabilities shines bright the larger the website.
Currently even with a small website this package still runs faster than the top OSS crawlers to date.

_Note_: Nodejs concurrency heavily impacts each additional run.
