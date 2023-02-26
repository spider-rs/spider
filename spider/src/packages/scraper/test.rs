use super::{Html, Selector};

#[test]
fn tag_with_newline() {
    let selector = Selector::parse("a").unwrap();

    let document = Html::parse_fragment(
        r#"
        <a
                            href="https://github.com/causal-agent/scraper">

                            </a>
        "#,
    );

    let mut iter = document.select(&selector);
    let a = iter.next().unwrap();
    assert_eq!(
        a.value().attr("href"),
        Some("https://github.com/causal-agent/scraper")
    );
}
