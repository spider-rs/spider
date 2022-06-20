# Benchmarks

## Table of Contents

- [Benchmark Results](#benchmark-results)
  - [crawl-speed](#crawl-speed)

## Benchmark Results

Specs of machine:

```
2-core CPU
7 GB of RAM memory
14 GB of SSD disk space
```

### crawl-speed

Target url `https://rsseau.fr`:

|                                          | `libraries`              |
| :--------------------------------------- | :----------------------- |
| **`Rust[spider]: crawl 10 samples`**     | `2.56 s` (✅ **1.00x**)  |
| **`Node.js[crawler]: crawl 10 samples`** | `3.46 s` (✅ **1.00x**)  |
| **`Go[crolly]: crawl 10 samples`**       | `4.97 s` (✅ **1.00x**)  |
| **`C[wget]: crawl 10 samples`**          | `18.71 s` (✅ **1.00x**) |
