# spider_transformations

The Rust spider cloud transformation library built for performance, AI, and multiple locales.
The library is used on [Spider Cloud](https://spider.cloud) for data cleaning.

## Usage

```toml
[dependencies]
spider_transformations = "0"
```

```rust
use spider_transformations::transformation::content;

fn main() {
    // page comes from the spider object when streaming.
    let conf = content::TransformConfig::default();
    let content = content::transform_content(&page, &conf);
}
```
### Transfrom types

1. Markdown
1. Commonmark
1. Text
1. Markdown (Text Map) or HTML2Text
1. WIP: HTML2XML

#### Enhancements

1. Readability
1. Encoding


## Chunking

There are several chunking utils in the transformation mod.

This project has rewrites and forks of html2md, and html2text for performance and bug fixes.