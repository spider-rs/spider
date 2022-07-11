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
| **`Rust[spider]: crawl 10 samples`**     | `2.8644 s` (✅ **10.00x**) |
| **`Go[crolly]: crawl 10 samples`**       | `4.2235 s` (✅ **10.00x**) |
| **`Node.js[crawler]: crawl 10 samples`** | `14.461 s` (✅ **10.00x**) |
| **`C[wget]: crawl 10 samples`**          | `16.181 s` (✅ **10.00x**) |
