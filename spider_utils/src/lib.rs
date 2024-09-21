use spider::packages::scraper::ElementRef;
use spider::tokio_stream::StreamExt;
use spider::utils::log;
use spider::{
    hashbrown::{hash_map::Entry, HashMap},
    packages::scraper::{Html, Selector},
    tokio,
};
use std::{fmt::Debug, hash::Hash};

#[cfg(feature = "transformations")]
pub use spider_transformations;

/// Extracted content from CSS query selectors.
type CSSQueryMap = HashMap<String, Vec<String>>;

/// Async stream CSS query selector map.
pub async fn css_query_select_map_streamed<K>(
    html: &str,
    selectors: &HashMap<K, Vec<Selector>>,
) -> CSSQueryMap
where
    K: AsRef<str> + Eq + Hash + Sized,
{
    let fragment = Html::parse_fragment(html);
    let mut stream = spider::tokio_stream::iter(selectors);
    let mut map: CSSQueryMap = HashMap::new();

    while let Some(selector) = stream.next().await {
        for s in selector.1 {
            for element in fragment.select(&s) {
                process_selector::<K>(element, &selector.0, &mut map);
            }
        }
    }

    map
}

/// Sync CSS query selector map.
pub fn css_query_select_map<K>(html: &str, selectors: &HashMap<K, Vec<Selector>>) -> CSSQueryMap
where
    K: AsRef<str> + Eq + Hash + Sized,
{
    let fragment = Html::parse_fragment(html);
    let mut map: CSSQueryMap = HashMap::new();

    for selector in selectors {
        for s in selector.1 {
            for element in fragment.select(&s) {
                process_selector::<K>(element, &selector.0, &mut map);
            }
        }
    }

    map
}

/// Process a single element and update the map with the results.
fn process_selector<K>(element: ElementRef, selector: &K, map: &mut CSSQueryMap)
where
    K: AsRef<str> + Eq + Hash + Sized,
{
    let name = selector.as_ref();
    let entry_name = if name.is_empty() {
        Default::default()
    } else {
        name.to_string()
    };

    let text = clean_element_text(&element);

    match map.entry(entry_name) {
        Entry::Occupied(mut entry) => entry.get_mut().push(text),
        Entry::Vacant(entry) => {
            entry.insert(vec![text]);
        }
    }
}

/// get the text extracted.
pub fn clean_element_text(element: &ElementRef) -> String {
    element.text().collect::<Vec<_>>().join(" ")
}

/// Build valid css selectors for extracting. The hashmap takes items with the key for the object key and the value is the css selector.
pub fn build_selectors_base<K, V, S>(selectors: HashMap<K, S>) -> HashMap<K, Vec<Selector>>
where
    K: AsRef<str> + Eq + Hash + Clone + Debug,
    V: AsRef<str> + Debug + AsRef<str>,
    S: IntoIterator<Item = V>,
{
    let mut valid_selectors: HashMap<K, Vec<Selector>> = HashMap::new();

    for (key, selector_set) in selectors {
        let mut selectors_vec = Vec::new();
        for selector_str in selector_set {
            match Selector::parse(selector_str.as_ref()) {
                Ok(selector) => selectors_vec.push(selector),
                Err(err) => log(
                    "",
                    format!(
                        "Failed to parse selector '{}': {:?}",
                        selector_str.as_ref(),
                        err
                    ),
                ),
            }
        }
        if !selectors_vec.is_empty() {
            valid_selectors.insert(key, selectors_vec);
        }
    }

    valid_selectors
}

/// Build valid css selectors for extracting. The hashmap takes items with the key for the object key and the value is the css selector.
#[cfg(not(feature = "indexset"))]
pub fn build_selectors<K, V>(
    selectors: HashMap<K, spider::hashbrown::HashSet<V>>,
) -> HashMap<K, Vec<Selector>>
where
    K: AsRef<str> + Eq + Hash + Clone + Debug,
    V: AsRef<str> + Debug + AsRef<str>,
{
    build_selectors_base::<K, V, spider::hashbrown::HashSet<V>>(selectors)
}

/// Build valid css selectors for extracting. The hashmap takes items with the key for the object key and the value is the css selector.
#[cfg(feature = "indexset")]
pub fn build_selectors<K, V>(
    selectors: HashMap<K, indexmap::IndexSet<V>>,
) -> HashMap<K, Vec<Selector>>
where
    K: AsRef<str> + Eq + Hash + Clone + Debug,
    V: AsRef<str> + Debug + AsRef<str>,
{
    build_selectors_base::<K, V, indexmap::IndexSet<V>>(selectors)
}

#[cfg(not(feature = "indexset"))]
pub type QueryCSSSelectSet<'a> = spider::hashbrown::HashSet<&'a str>;
#[cfg(feature = "indexset")]
pub type QueryCSSSelectSet<'a> = indexmap::IndexSet<&'a str>;
#[cfg(not(feature = "indexset"))]
pub type QueryCSSMap<'a> = HashMap<&'a str, QueryCSSSelectSet<'a>>;
#[cfg(feature = "indexset")]
pub type QueryCSSMap<'a> = HashMap<&'a str, QueryCSSSelectSet<'a>>;

#[tokio::test]
async fn test_css_query_select_map_streamed() {
    let map = QueryCSSMap::from([("list", QueryCSSSelectSet::from([".list", ".sub-list"]))]);

    let data = css_query_select_map_streamed(
        r#"<html><body><ul class="list"><li>Test</li></ul></body></html>"#,
        &build_selectors(map),
    )
    .await;

    assert!(!data.is_empty(), "CSS extraction failed",);
}

#[test]
fn test_css_query_select_map() {
    let map = QueryCSSMap::from([("list", QueryCSSSelectSet::from([".list", ".sub-list"]))]);
    let data = css_query_select_map(
        r#"<html><body><ul class="list">Test</ul></body></html>"#,
        &build_selectors(map),
    );

    assert!(!data.is_empty(), "CSS extraction failed",);
}

#[tokio::test]
async fn test_css_query_select_map_streamed_multi_join() {
    let map = QueryCSSMap::from([("list", QueryCSSSelectSet::from([".list", ".sub-list"]))]);
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

    assert!(!data.is_empty(), "CSS extraction failed");
}
