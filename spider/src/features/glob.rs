use crate::CompactString;

#[cfg(feature = "glob")]
pub fn expand_url(url: &str) -> Vec<CompactString> {
    use itertools::Itertools;
    use regex::Regex;
    use urlencoding::decode;

    lazy_static! {
        static ref RE: Regex = {
            regex::Regex::new(
                r"(?x)
                    # list
                    (?<list>\{(?<items>[^}\\}^\{]+)}) |
                    # range
                    (?<range>\[(?:(?<start>(?<padding>0*)\d+|[a-z]))-(?:(?<end>\d+|[a-z]))(?::(?<step>\d+))?])
                ",
            )
            .unwrap()
        };
    }

    let mut matches = Vec::new();

    let url: CompactString = match decode(url) {
        Ok(u) => u.into(),
        _ => url.into(),
    };

    for capture in RE.captures_iter(&url) {
        match (
            capture.name("list"),
            capture.name("items"),
            capture.name("range"),
            capture.name("start"),
            capture.name("end"),
        ) {
            // matches a list
            (Some(list), Some(items), _, _, _) => {
                let substring = list.as_str();

                let items = items
                    .as_str()
                    .split(",")
                    .map(|item| (item.to_string(), substring))
                    .collect::<Vec<(String, &str)>>();

                matches.push(items);
            }
            // matches a range
            (_, _, Some(range), Some(start), Some(end)) => {
                let substring = range.as_str();
                let step = match capture.name("step") {
                    Some(step) => step.as_str().parse::<usize>().unwrap(),
                    None => 1,
                };
                let start_str = start.as_str();
                let end_str = end.as_str();

                let width = match capture.name("padding") {
                    Some(padding) => {
                        if padding.as_str().len() > 0 {
                            start_str.len()
                        } else {
                            0
                        }
                    }
                    None => 0,
                };

                match (start_str.parse::<u32>(), end_str.parse::<u32>()) {
                    // start and end are numbers
                    (Ok(s), Ok(e)) => {
                        let items = (s..e + 1)
                            .step_by(step)
                            .map(|num| {
                                (
                                    format!("{:0>width$}", num.to_string(), width = width),
                                    substring,
                                )
                            })
                            .collect::<Vec<(String, &str)>>();

                        matches.push(items);
                    }
                    // start and end are characters
                    _ => {
                        let s = start_str.as_bytes()[0];
                        let e = end_str.as_bytes()[0];
                        let items = (s..e + 1)
                            .map(|char| (String::from_utf8_lossy(&[char]).to_string(), substring))
                            .collect::<Vec<(String, &str)>>();

                        matches.push(items);
                    }
                };
            }
            _ => {}
        }
    }

    matches
        .into_iter()
        .multi_cartesian_product()
        .map(|combination| {
            let mut new_url = url.clone();

            for (replacement, substring) in combination {
                new_url = new_url.replace(substring, replacement.as_str()).into();
            }

            new_url
        })
        .collect::<Vec<CompactString>>()
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_list() {
    let url = "https://choosealicense.com/licenses/{mit,apache-2.0,mpl-2.0}/";

    assert_eq!(
        expand_url(url),
        [
            "https://choosealicense.com/licenses/mit/",
            "https://choosealicense.com/licenses/apache-2.0/",
            "https://choosealicense.com/licenses/mpl-2.0/"
        ]
    );
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_list_escaped_closing() {
    let url = "https://choosealicense.com/licenses/{mit\\}/";

    assert_eq!(expand_url(url), Vec::<CompactString>::new());
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_numerical_range() {
    let url = "https://choosealicense.com/licenses/bsd-[2-4]-clause/";

    assert_eq!(
        expand_url(url),
        [
            "https://choosealicense.com/licenses/bsd-2-clause/",
            "https://choosealicense.com/licenses/bsd-3-clause/",
            "https://choosealicense.com/licenses/bsd-4-clause/",
        ]
    );
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_numerical_range_singe_item() {
    let url = "https://choosealicense.com/licenses/bsd-[4-4]-clause/";

    assert_eq!(
        expand_url(url),
        ["https://choosealicense.com/licenses/bsd-4-clause/"]
    );
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_numerical_range_with_step() {
    let url = "https://choosealicense.com/licenses/bsd-[2-4:2]-clause/";

    assert_eq!(
        expand_url(url),
        [
            "https://choosealicense.com/licenses/bsd-2-clause/",
            "https://choosealicense.com/licenses/bsd-4-clause/",
        ]
    );
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_numerical_range_with_padding() {
    let url = "https://choosealicense.com/licenses/bsd-[002-004]-clause/";

    assert_eq!(
        expand_url(url),
        [
            "https://choosealicense.com/licenses/bsd-002-clause/",
            "https://choosealicense.com/licenses/bsd-003-clause/",
            "https://choosealicense.com/licenses/bsd-004-clause/",
        ]
    );
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_numerical_range_with_padding_ignore_end_padding() {
    let url = "https://choosealicense.com/licenses/bsd-[008-10]-clause/";

    assert_eq!(
        expand_url(url),
        [
            "https://choosealicense.com/licenses/bsd-008-clause/",
            "https://choosealicense.com/licenses/bsd-009-clause/",
            "https://choosealicense.com/licenses/bsd-010-clause/",
        ]
    );
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_alphabetical_range() {
    let url = "https://choosealicense.com/licenses/[w-z]lib/";

    assert_eq!(
        expand_url(url),
        [
            "https://choosealicense.com/licenses/wlib/",
            "https://choosealicense.com/licenses/xlib/",
            "https://choosealicense.com/licenses/ylib/",
            "https://choosealicense.com/licenses/zlib/",
        ]
    );
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_combination() {
    let url = "https://choosealicense.com/licenses/bsd-[2-4]-clause{,-clear}/";

    assert_eq!(
        expand_url(url),
        [
            "https://choosealicense.com/licenses/bsd-2-clause/",
            "https://choosealicense.com/licenses/bsd-2-clause-clear/",
            "https://choosealicense.com/licenses/bsd-3-clause/",
            "https://choosealicense.com/licenses/bsd-3-clause-clear/",
            "https://choosealicense.com/licenses/bsd-4-clause/",
            "https://choosealicense.com/licenses/bsd-4-clause-clear/",
        ]
    );
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_empty() {
    let url = "https://choosealicense.com";

    assert_eq!(expand_url(url), Vec::<CompactString>::new());
}
