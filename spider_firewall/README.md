# spider_firewall

A shield to prevent bad websites from messing up your system.

`cargo add spider_firewall`

```rust
use spider_firewall::is_bad_website_url;

fn main() {
    let domain = url::Url::parse("https://badwebsite.com").expect("parse");
    let blocked = is_bad_website_url(&domain);
}
```

TODO:

1. We can use something like `https://github.com/ShadowWhisperer/BlockLists` and other sources to compile a dynamic list to pull from.
1. Add a cron method to recompile the list at runtime with sources daily.
1. Fix the build script to pull only text files without the custom shape `"www.something",` and allow simple multi line parsing.