extern crate spectral;

use super::html2md::parse_html;
use std::fs::File;
use std::io::prelude::*;

use spectral::prelude::*;
use indoc::indoc;

#[test]
#[ignore]
fn test_marcfs() {
    let mut html = String::new();
    let mut html_file = File::open("test-samples/marcfs-readme.html").unwrap();
    html_file.read_to_string(&mut html).expect("File must be readable");
    let result = parse_html(&html);
    println!("{}", result);
}

#[test]
#[ignore]
fn test_cheatsheet() {
    let mut html = String::new();
    let mut md = String::new();
    let mut html_file = File::open("test-samples/markdown-cheatsheet.html").unwrap();
    let mut md_file = File::open("test-samples/markdown-cheatsheet.md").unwrap();
    html_file.read_to_string(&mut html).expect("File must be readable");
    md_file.read_to_string(&mut md).expect("File must be readable");
    let md_parsed = parse_html(&html);
    println!("{}", md_parsed);
    //assert_eq!(md, md_parsed);
}

/// newlines after list shouldn't be converted into text of the last list element
#[test]
fn test_list_newlines() {
    let mut html = String::new();
    let mut html_file = File::open("test-samples/dybr-bug-with-list-newlines.html").unwrap();
    html_file.read_to_string(&mut html).expect("File must be readable");
    let result = parse_html(&html);
    assert_that(&result).contains(".\n\nxxx xxxx");
    assert_that(&result).contains("xx x.\n\nxxxxx:");
}


#[test]
fn test_lists_from_text() {
    let mut html = String::new();
    let mut html_file = File::open("test-samples/dybr-bug-with-lists-from-text.html").unwrap();
    html_file.read_to_string(&mut html).expect("File must be readable");
    let result = parse_html(&html);
    assert_that(&result).contains("\\- x xxxx xxxxx xx xxxxxxxxxx");
    assert_that(&result).contains("\\- x xxxx xxxxxxxx xxxxxxxxx xxxxxx xxx x xxxxxxxx xxxx");
    assert_that(&result).contains("\\- xxxx xxxxxxxx");
}

#[test]
fn test_strong_inside_link() {
    let mut html = String::new();
    let mut html_file = File::open("test-samples/dybr-bug-with-strong-inside-link.html").unwrap();
    html_file.read_to_string(&mut html).expect("File must be readable");
    let result = parse_html(&html);
    assert_that(&result).contains("[**Just God**](http://fanfics.me/ficXXXXXXX)");
}

#[test]
fn test_tables_with_newlines() {
    let mut html = String::new();
    let mut html_file = File::open("test-samples/dybr-bug-with-tables-masked.html").unwrap();
    html_file.read_to_string(&mut html).expect("File must be readable");
    let result = parse_html(&html);

    // all lines starting with | should end with | as well
    let invalid_table_lines: Vec<&str> = result.lines()
        .filter(|line| line.starts_with("|"))
        .filter(|line| !line.ends_with("|"))
        .collect();

    assert_that(&invalid_table_lines).is_empty();
}

#[test]
fn test_tables_crash2() {
    let mut html = String::new();
    let mut html_file = File::open("test-samples/dybr-bug-with-tables-2-masked.html").unwrap();
    html_file.read_to_string(&mut html).expect("File must be readable");
    let table_with_vertical_header = parse_html(&html);

    assert_that!(table_with_vertical_header).contains(indoc! {"
        |Current Conditions:|Open all year. No reservations. No services.|
        |-------------------|--------------------------------------------|
        |   Reservations:   |              No reservations.              |
        |       Fees        |                  No fee.                   |
        |      Water:       |                 No water.                  |"
    });
}