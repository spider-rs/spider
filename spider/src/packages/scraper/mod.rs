//! HTML parsing and querying with CSS selectors.
//!
//! `crate::packages::scraper` is on [Crates.io][crate] and [GitHub][github].
//!
//! [crate]: https://crates.io/crates/scraper
//! [github]: https://github.com/programble/scraper
//!
//! Scraper provides an interface to Servo's `html5ever` and `selectors` crates, for browser-grade
//! parsing and querying.
//!
//! # Examples
//!
//! ## Parsing a document
//!
//! ```
//! use spider::packages::scraper::Html;
//!
//! let html = r#"
//!     <!DOCTYPE html>
//!     <meta charset="utf-8">
//!     <title>Hello, world!</title>
//!     <h1 class="foo">Hello, <i>world!</i></h1>
//! "#;
//!
//! let document = Html::parse_document(html);
//! ```
//!
//! ## Parsing a fragment
//!
//! ```
//! use spider::packages::scraper::Html;
//! let fragment = Html::parse_fragment("<h1>Hello, <i>world!</i></h1>");
//! ```
//!
//! ## Parsing a selector
//!
//! ```
//! use spider::packages::scraper::Selector;
//! let selector = Selector::parse("h1.foo").unwrap();
//! ```
//!
//! ## Selecting elements
//!
//! ```
//! use spider::packages::scraper::{Html, Selector};
//!
//! let html = r#"
//!     <ul>
//!         <li>Foo</li>
//!         <li>Bar</li>
//!         <li>Baz</li>
//!     </ul>
//! "#;
//!
//! let fragment = Html::parse_fragment(html);
//! let selector = Selector::parse("li").unwrap();
//!
//! for element in fragment.select(&selector) {
//!     assert_eq!("li", element.value().name());
//! }
//! ```
//!
//! ## Selecting descendent elements
//!
//! ```
//! use spider::packages::scraper::{Html, Selector};
//!
//! let html = r#"
//!     <ul>
//!         <li>Foo</li>
//!         <li>Bar</li>
//!         <li>Baz</li>
//!     </ul>
//! "#;
//!
//! let fragment = Html::parse_fragment(html);
//! let ul_selector = Selector::parse("ul").unwrap();
//! let li_selector = Selector::parse("li").unwrap();
//!
//! let ul = fragment.select(&ul_selector).next().unwrap();
//! for element in ul.select(&li_selector) {
//!     assert_eq!("li", element.value().name());
//! }
//! ```
//!
//! ## Accessing element attributes
//!
//! ```
//! use spider::packages::scraper::{Html, Selector};
//!
//! let fragment = Html::parse_fragment(r#"<input name="foo" value="bar">"#);
//! let selector = Selector::parse(r#"input[name="foo"]"#).unwrap();
//!
//! let input = fragment.select(&selector).next().unwrap();
//! assert_eq!(Some("bar"), input.value().attr("value"));
//! ```
//!
//! ## Serializing HTML and inner HTML
//!
//! ```
//! use spider::packages::scraper::{Html, Selector};
//!
//! let fragment = Html::parse_fragment("<h1>Hello, <i>world!</i></h1>");
//! let selector = Selector::parse("h1").unwrap();
//!
//! let h1 = fragment.select(&selector).next().unwrap();
//!
//! assert_eq!("<h1>Hello, <i>world!</i></h1>", h1.html());
//! assert_eq!("Hello, <i>world!</i>", h1.inner_html());
//! ```
//!
//! ## Accessing descendent text
//!
//! ```
//! use spider::packages::scraper::{Html, Selector};
//!
//! let fragment = Html::parse_fragment("<h1>Hello, <i>world!</i></h1>");
//! let selector = Selector::parse("h1").unwrap();
//!
//! let h1 = fragment.select(&selector).next().unwrap();
//! let text = h1.text().collect::<Vec<_>>();
//!
//! assert_eq!(vec!["Hello, ", "world!"], text);
//! ```

pub use element_ref::ElementRef;
pub use html::Html;
pub use node::Node;
pub use selector::Selector;
pub use selectors::Element;

pub mod element_ref;
pub mod error;
pub mod html;
pub mod node;
pub mod selector;

#[cfg(test)]
mod test;
