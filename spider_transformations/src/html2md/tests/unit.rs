use super::html2md::parse_html;
use pretty_assertions::assert_eq;

#[test]
fn test_dumb() {
    let md = parse_html("<p>CARTHAPHILUS</p>");
    assert_eq!(md, "CARTHAPHILUS")
}

#[test]
// fixme
fn test_space() {
    let md = parse_html(r#"<p><a href="http://ya.ru">APOSIMZ</a></p>\n"#);
    assert_eq!(md, "[APOSIMZ](http://ya.ru)\n\n\\\\n")
}

#[test]
fn test_anchor() {
    let md = parse_html(r#"<p><a href="http://ya.ru">APOSIMZ</a></p>"#);
    assert_eq!(md, "[APOSIMZ](http://ya.ru)")
}

#[test]
fn test_anchor2() {
    let md = parse_html(r#"<p><a href="http://ya.ru">APOSIMZ</a><a href="http://yandex.ru">SIDONIA</a></p>"#);
    assert_eq!(md, "[APOSIMZ](http://ya.ru)[SIDONIA](http://yandex.ru)")
}

#[test]
fn test_anchor3() {
    let md = parse_html(r#"<p><a href="http://ya.ru">APOSIMZ</a><p/><a href="http://yandex.ru">SIDONIA</a></p>"#);
    assert_eq!(md, "\
[APOSIMZ](http://ya.ru)

[SIDONIA](http://yandex.ru)")
}

#[test]
fn test_image() {
    let md = parse_html(r#"<p><a href="https://gitter.im/MARC-FS/Lobby?utm_source=badge&amp;utm_medium=badge&amp;utm_campaign=pr-badge&amp;utm_content=badge"><img src="https://img.shields.io/gitter/room/MARC-FS/MARC-FS.svg" alt="Gitter"></a><br>"#);
    assert_eq!(md, "[![Gitter](https://img.shields.io/gitter/room/MARC-FS/MARC-FS.svg)](https://gitter.im/MARC-FS/Lobby?utm_source=badge&utm_medium=badge&utm_campaign=pr-badge&utm_content=badge)")
}

#[test]
fn test_escaping() {
    let md = parse_html(r#"<p>*god*'s in his **heaven** - all is right with the __world__</p>"#);
    assert_eq!(md, "\\*god\\*\'s in his \\*\\*heaven\\*\\* - all is right with the \\_\\_world\\_\\_")
}

#[test]
fn test_escaping_mid_hyphens() {
    let md = parse_html(r#"<h1>This is a header with-hyphen!</h1>"#);
    assert_eq!(md, "This is a header with-hyphen!\n==========")
}

#[test]
fn test_escaping_start_hyphens() {
    let md = parse_html(r#"<h1>- This is a header with starting hyphen!</h1>"#);
    assert_eq!(md, "\\- This is a header with starting hyphen!\n==========")
}

#[test]
fn test_escaping_start_sharp() {
    let md = parse_html("<html># nothing to worry about</html>");
    assert_eq!(md, "\\# nothing to worry about")
}

/// Note: Also strips multiple spaces
#[test]
fn test_escaping_start_hyphens_space() {
    let md = parse_html(r#"<h1>   - This is a header with starting hyphen!</h1>"#);
    assert_eq!(md, " \\- This is a header with starting hyphen!\n==========")
}

#[test]
fn test_escaping_html_tags() {
    let md = parse_html(r#"xxxxxxx xx xxxxxxxxxxx: &lt;iframe src="xxxxxx_xx_xxxxxxxxxxx/embed/" allowfullscreen="" height="725" width="450"&gt;&lt;/iframe&gt;"#);
    assert_eq!(md, r#"xxxxxxx xx xxxxxxxxxxx: \<iframe src="xxxxxx\_xx\_xxxxxxxxxxx/embed/" allowfullscreen="" height="725" width="450"\>\</iframe\>"#)
}

#[test]
fn test_headers() {
    let md = parse_html(r#"<h1 id="marc-fs">MARC-FS</h1><p><a href="http://Mail.ru">Mail.ru</a> Cloud filesystem written for FUSE</p><h2 id="synopsis">Synopsis</h2>"#);
    assert_eq!(md, "\
MARC-FS
==========

[Mail.ru](http://Mail.ru) Cloud filesystem written for FUSE

Synopsis
----------")
}

#[test]
fn test_escaping_start_equal() {
    let md = parse_html(r#"<p>This is NOT a header!<br/>===========</p>"#);
    assert_eq!(md, "This is NOT a header!  \n\\===========")
}

/// Note: Also strips multiple spaces
#[test]
fn test_escaping_start_equal_space() {
    let md = parse_html(r#"<p>This is NOT a header!<br/>  ===========</p>"#);
    assert_eq!(md, "This is NOT a header!  \n \\===========")
}

#[test]
fn test_escaping_start_hyphen() {
    let md = parse_html(r#"<p>This is NOT a header!<br/>-------</p>"#);
    assert_eq!(md, "This is NOT a header!  \n\\-------")
}

/// Note: Also strips multiple spaces
#[test]
fn test_escaping_start_hyphen_space() {
    let md = parse_html(r#"<p>This is NOT a header!<br/>     -------</p>"#);
    assert_eq!(md, "This is NOT a header!  \n \\-------")
}