# spider_utils

Utilities to use to help with getting the most out of spider.

## CSS Scraping

```rust
use spider::{
    hashbrown::HashMap,
    packages::scraper::Selector,
};
use spider_utils::{QueryCSSMap, QueryCSSSelectSet, build_selectors, css_query_select_map_streamed};

async fn css_query_selector_extract() {
    let map = QueryCSSMap::from([(
        "list",
        QueryCSSSelectSet::from([".list", ".sub-list"]),
    )]);
    let data = css_query_select_map_streamed(
        r#"<html>
            <body>
                <ul class="list"><li>First</li></ul>
                <ul class="sub-list"><li>Second</li></ul>
            </body>
        </html>"#,
        &build_selectors(map),
    )
    .await;

    println!("{:?}", data);
    // {"list": ["First", "Second"]}
}
```

## Features

You can use the feature flag `indexset` to order the CSS scraping extraction order.