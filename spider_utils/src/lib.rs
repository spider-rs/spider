use scraper::{ElementRef, Html, Selector};
use spider::lazy_static::lazy_static;
use spider::tokio_stream::StreamExt;
use spider::utils::log;
use spider::{
    hashbrown::{hash_map::Entry, HashMap},
    tokio,
};
use std::{fmt::Debug, hash::Hash};
use sxd_document::parser;
use sxd_xpath::evaluate_xpath;

/// The type of selectors that can be used to query.
#[derive(Default, Debug, Clone)]
pub struct DocumentSelectors<K> {
    /// CSS Selectors.
    pub css: HashMap<K, Vec<Selector>>,
    /// XPath Selectors.
    pub xpath: HashMap<K, Vec<String>>,
}

#[cfg(feature = "transformations")]
pub use spider_transformations;

/// Extracted content from CSS query selectors.
type CSSQueryMap = HashMap<String, Vec<String>>;

/// Check if a selector is a valid xpath
fn is_valid_xpath(expression: &str) -> bool {
    use sxd_xpath::Factory;
    lazy_static! {
        static ref XPATH_FACTORY: Factory = Factory::new();
    };
    match XPATH_FACTORY.build(expression) {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(_) => false,
    }
}

/// Async stream CSS query selector map.
pub async fn css_query_select_map_streamed<K>(
    html: &str,
    selectors: &DocumentSelectors<K>,
) -> CSSQueryMap
where
    K: AsRef<str> + Eq + Hash + Sized,
{
    let mut map: CSSQueryMap = HashMap::new();

    if !selectors.css.is_empty() {
        let mut stream = spider::tokio_stream::iter(&selectors.css);
        let fragment = Box::new(Html::parse_fragment(html));

        while let Some(selector) = stream.next().await {
            for s in selector.1 {
                for element in fragment.select(s) {
                    process_selector::<K>(element, selector.0, &mut map);
                }
            }
        }
    }

    if !selectors.xpath.is_empty() {
        if let Ok(package) = parser::parse(html) {
            let document = Box::new(package.as_document());

            for selector in selectors.xpath.iter() {
                for s in selector.1 {
                    if let Ok(value) = evaluate_xpath(&document, s) {
                        let text = value.into_string();

                        if !text.is_empty() {
                            match map.entry(selector.0.as_ref().to_string()) {
                                Entry::Occupied(mut entry) => entry.get_mut().push(text),
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![text]);
                                }
                            }
                        }
                    };
                }
            }
        };
    }

    for items in map.values_mut() {
        items.dedup();
    }

    map
}

/// Sync CSS query selector map.
pub fn css_query_select_map<K>(html: &str, selectors: &DocumentSelectors<K>) -> CSSQueryMap
where
    K: AsRef<str> + Eq + Hash + Sized,
{
    let mut map: CSSQueryMap = HashMap::new();

    if !selectors.css.is_empty() {
        let fragment = Box::new(Html::parse_fragment(html));

        for selector in selectors.css.iter() {
            for s in selector.1 {
                for element in fragment.select(s) {
                    process_selector::<K>(element, selector.0, &mut map);
                }
            }
        }
    }

    if !selectors.xpath.is_empty() {
        if let Ok(package) = parser::parse(html) {
            let document = package.as_document();

            for selector in selectors.xpath.iter() {
                for s in selector.1 {
                    if let Ok(value) = evaluate_xpath(&document, s) {
                        let text = value.into_string();

                        if !text.is_empty() {
                            match map.entry(selector.0.as_ref().to_string()) {
                                Entry::Occupied(mut entry) => entry.get_mut().push(text),
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![text]);
                                }
                            }
                        }
                    };
                }
            }
        };
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

    if !text.is_empty() {
        match map.entry(entry_name) {
            Entry::Occupied(mut entry) => entry.get_mut().push(text),
            Entry::Vacant(entry) => {
                entry.insert(vec![text]);
            }
        }
    }
}

/// get the text extracted.
pub fn clean_element_text(element: &ElementRef) -> String {
    element.text().collect::<Vec<_>>().join(" ")
}

/// Build valid css selectors for extracting. The hashmap takes items with the key for the object key and the value is the css selector.
pub fn build_selectors_base<K, V, S>(selectors: HashMap<K, S>) -> DocumentSelectors<K>
where
    K: AsRef<str> + Eq + Hash + Clone + Debug,
    V: AsRef<str> + Debug + AsRef<str>,
    S: IntoIterator<Item = V>,
{
    let mut valid_selectors: HashMap<K, Vec<Selector>> = HashMap::new();
    let mut valid_selectors_xpath: HashMap<K, Vec<String>> = HashMap::new();

    for (key, selector_set) in selectors {
        let mut selectors_vec = Vec::new();
        let mut selectors_vec_xpath = Vec::new();

        for selector_str in selector_set {
            match Selector::parse(selector_str.as_ref()) {
                Ok(selector) => selectors_vec.push(selector),
                Err(err) => {
                    if is_valid_xpath(selector_str.as_ref()) {
                        selectors_vec_xpath.push(selector_str.as_ref().to_string())
                    } else {
                        log(
                            "",
                            format!(
                                "Failed to parse selector '{}': {:?}",
                                selector_str.as_ref(),
                                err
                            ),
                        )
                    }
                }
            }
        }

        let has_css_selectors = !selectors_vec.is_empty();
        let has_xpath_selectors = !selectors_vec_xpath.is_empty();

        if has_css_selectors && !has_xpath_selectors {
            valid_selectors.insert(key, selectors_vec);
        } else if !has_css_selectors && has_xpath_selectors {
            valid_selectors_xpath.insert(key, selectors_vec_xpath);
        } else {
            if has_css_selectors {
                valid_selectors.insert(key.clone(), selectors_vec);
            }
            if has_xpath_selectors {
                valid_selectors_xpath.insert(key, selectors_vec_xpath);
            }
        }
    }

    DocumentSelectors {
        css: valid_selectors,
        xpath: valid_selectors_xpath,
    }
}

/// Build valid css selectors for extracting. The hashmap takes items with the key for the object key and the value is the css selector.
#[cfg(not(feature = "indexset"))]
pub fn build_selectors<K, V>(
    selectors: HashMap<K, spider::hashbrown::HashSet<V>>,
) -> DocumentSelectors<K>
where
    K: AsRef<str> + Eq + Hash + Clone + Debug,
    V: AsRef<str> + Debug + AsRef<str>,
{
    build_selectors_base::<K, V, spider::hashbrown::HashSet<V>>(selectors)
}

/// Build valid css selectors for extracting. The hashmap takes items with the key for the object key and the value is the css selector.
#[cfg(feature = "indexset")]
pub fn build_selectors<K, V>(selectors: HashMap<K, indexmap::IndexSet<V>>) -> DocumentSelectors<K>
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

#[tokio::test]
async fn test_xpath_query_select_map_streamed() {
    let map = QueryCSSMap::from([(
        "list",
        QueryCSSSelectSet::from(["//*[@class='list']", "//*[@class='sub-list']"]),
    )]);
    let selectors = build_selectors(map);
    let data = css_query_select_map_streamed(
        r#"<html><body><ul class="list"><li>Test</li></ul></body></html>"#,
        &selectors,
    )
    .await;

    assert!(!data.is_empty(), "Xpath extraction failed",);
}
