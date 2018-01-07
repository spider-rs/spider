extern crate reqwest;
extern crate scraper;

use std::io::Read;
use scraper::{Html, Selector};

/// Launch an HTTP GET query to te given URL & parse body response content
fn get(url: &str) -> Html {
    let mut res = reqwest::get(url).unwrap();
    let mut body = String::new();
    res.read_to_string(&mut body).unwrap();

    Html::parse_document(&body)
}


fn main() {
    let body: Html = get("http://localhost:4000");
    let selector = Selector::parse("a").unwrap();

    for element in body.select(&selector) {
        assert_eq!("a", element.value().name());
        println!("{:?}", element.value());
    }
}
