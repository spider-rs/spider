# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
  - [crawl-speed](#crawl-speed)

## Benchmark Results

### AMD

```sh
----------------------
linux ubuntu-latest
2-core CPU
7 GB of RAM memory
14 GB of SSD disk space
-----------------------
Url: `https://rsseau.fr` locally.

15 pages
```

#### crawl-speed

runs with 10 samples:

|                                          | `libraries`                |
| :--------------------------------------- | :------------------------- |
| **`Rust[spider]: crawl 10 samples`**     | `44.130 ms` (✅ **1.00x**) |
| **`Go[crolly]: crawl 10 samples`**       | `412.75 ms` (✅ **1.00x**) |
| **`Node.js[crawler]: crawl 10 samples`** | `506.01 ms` (✅ **1.00x**) |
| **`C[wget]: crawl 10 samples`**          | `44.592 ms` (✅ **1.00x**) |

#### crawl-speed-concurrentx10

10 concurrent runs with 10 samples:

|                                          | `libraries`                 |
| :--------------------------------------- | :-------------------------- |
| **`Rust[spider]: crawl 10 samples`**     | `312.56 ms` (✅ **10.00x**) |
| **`Go[crolly]: crawl 10 samples`**       | `753.39 ms` (✅ **10.00x**) |
| **`Node.js[crawler]: crawl 10 samples`** | `3.2587 s` (✅ **10.00x**)  |
| **`C[wget]: crawl 10 samples`**          | `347.13 ms` (✅ **10.00x**) |

### Arm64

```sh
----------------------
MacBookPro18,2 Apple M1 Max
10-core CPU
64 GB of RAM memory
1 TB of SSD disk space
-----------------------
Url: `https://rsseau.fr` locally.

15 pages
```

#### crawl-speed

|                                          | `libraries`                |
| :--------------------------------------- | :------------------------- |
| **`Rust[spider]: crawl 10 samples`**     | `5.66 ms` (✅ **1.00x**)   |
| **`Go[crolly]: crawl 10 samples`**       | `9.74 s` (✅ **1.00x**)    |
| **`Node.js[crawler]: crawl 10 samples`** | `388.07 ms` (✅ **1.00x**) |
| **`C[wget]: crawl 10 samples`**          | `51.434 ms` (✅ **1.00x**) |

#### crawl-speed-concurrentx10

|                                                     | `libraries`                |
| :-------------------------------------------------- | :------------------------- |
| **`Rust[spider]: crawl concurrent 10 samples`**     | `279.67 ms` (✅ **1.00x**) |
| **`Go[crolly]: crawl concurrent 10 samples`**       | `10.47 s` (✅ **1.00x**)   |
| **`Node.js[crawler]: crawl concurrent 10 samples`** | `658.82 ms` (✅ **1.00x**) |
| **`C[wget]: crawl concurrent 10 samples`**          | `58.434 ms` (✅ **1.00x**) |

The concurrent benchmarks are averaged across 10 individual runs for 10 concurrent crawls with 10 sample counts.

In order for us to get better metrics we need to test the concurrency and simultaneous runs with a larger website and favorably a website that can spin up inside the local container so latency is not being tracked. The multi-threaded crawling capabilities shines bright the larger the website.
Currently even with a small website this package still runs faster than the top OSS crawlers to date.

_*Note*_

Nodejs concurrency heavily impacts each additional run when networking is enabled and tested on a non local website.

Wget performance is impacted when using networking outside of local connections.
