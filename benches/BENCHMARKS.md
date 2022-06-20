# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
  - [crawl-speed](#crawl-speed)

## Benchmark Results

```sh
----------------------
2-core CPU
7 GB of RAM memory
14 GB of SSD disk space
-----------------------

Test url: `https://rsseau.fr`
```

### crawl-speed

|                                          | `libraries`               |
| :--------------------------------------- | :------------------------ |
| **`Rust[spider]: crawl 10 samples`**     | `1.8375 s` (✅ **1.00x**) |
| **`Go[crolly]: crawl 10 samples`**       | `2.9417 s` (✅ **1.00x**) |
| **`Node.js[crawler]: crawl 10 samples`** | `2.9992 s` (✅ **1.00x**) |
| **`C[wget]: crawl 10 samples`**          | `12.019 s` (✅ **1.00x**) |
