use super::html2md::parse_html;
use pretty_assertions::assert_eq;
use indoc::indoc;

#[test]
fn test_quotes() {
    let md = parse_html("<p><blockquote>here's a quote\n next line of it</blockquote>And some text after it</p>");
    assert_eq!(md, "\
> here's a quote next line of it

And some text after it")
}

#[test]
fn test_quotes2() {
    let md = parse_html("<p><blockquote>here's<blockquote>nested quote!</blockquote> a quote\n next line of it</blockquote></p>");
    assert_eq!(md, "\
> here's
> > nested quote!
>
>  a quote next line of it")
}

#[test]
fn test_blockquotes() {
    let md = parse_html("<blockquote>Quote at the start of the message</blockquote>Should not crash the parser");
    assert_eq!(md, "\
> Quote at the start of the message

Should not crash the parser")
}

#[test]
fn test_details() {
    let html = indoc! {"
    <details>
        <summary>There are more things in heaven and Earth, <b>Horatio</b></summary>
        <p>Than are dreamt of in your philosophy</p>
    </details>
    "};
    let md = parse_html(&html);
    assert_eq!(md, "<details> <summary>There are more things in heaven and Earth, **Horatio**</summary>\n\nThan are dreamt of in your philosophy\n\n</details>")
}

#[test]
fn test_subsup() {
    let md = parse_html("X<sub>2</sub>");
    assert_eq!(md, r#"X<sub>2</sub>"#)
}