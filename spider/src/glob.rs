use itertools::Itertools;

#[cfg(feature = "glob")]
pub fn expand_url(url: &str) -> Vec<String> {
    let mut matches = Vec::new();

    for capture in regex::Regex::new(r"(?x)
    (?<list>\{(?<items>[^}]+)}) |  # list
    (?<range>\[(?:(?<start>\d+|\w))-(?:(?<end>\d+|\w))(?::(?<step>\d+))?])  # range
    ").unwrap().captures_iter(url) {
        match capture.name("list") {
            Some(list) => {
                let substring = list.as_str();
                let items = capture.name("items").unwrap().as_str().split(",").map(|item| (item.to_string(), substring)).collect::<Vec<(String, &str)>>();
                matches.push(items);
                continue;
            }
            _ => {}
        }
        match capture.name("range") {
            Some(range) => {
                let substring = range.as_str();
                let step = match capture.name("step") {
                    Some(step) => {step.as_str().parse::<usize>().unwrap()}
                    None => {1}
                };

                match (capture.name("start"),  capture.name("end")) {
                    (Some(start), Some(end)) => {
                        let s = start.as_str().parse::<u32>().unwrap();
                        let e = end.as_str().parse::<u32>().unwrap();
                        let items = (s..e+1).step_by(step).map(|num| (num.to_string(), substring)).collect::<Vec<(String, &str)>>();
                        matches.push(items);
                    }
                    _ => {}
                };
            }
            _ => {}
        }
    }

    matches.into_iter().multi_cartesian_product().map(|combination| {
        let mut new_url = String::from(url);
        for (replacement, substring) in combination {
            new_url = new_url.replace(substring, replacement.as_str());
        };
        new_url
    }).collect::<Vec<String>>()
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_list() {
    let url = "https://choosealicense.com/licenses/{mit,apache-2.0,mpl-2.0}/";

    assert_eq!(expand_url(url), [
        "https://choosealicense.com/licenses/mit/",
        "https://choosealicense.com/licenses/apache-2.0/",
        "https://choosealicense.com/licenses/mpl-2.0/"
    ]);
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_range() {
    let url = "https://choosealicense.com/licenses/bsd-[2-4]-clause/";

    assert_eq!(expand_url(url), [
        "https://choosealicense.com/licenses/bsd-2-clause/",
        "https://choosealicense.com/licenses/bsd-3-clause/",
        "https://choosealicense.com/licenses/bsd-4-clause/",
    ]);
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_range_with_step() {
    let url = "https://choosealicense.com/licenses/bsd-[2-4:2]-clause/";

    assert_eq!(expand_url(url), [
        "https://choosealicense.com/licenses/bsd-2-clause/",
        "https://choosealicense.com/licenses/bsd-4-clause/",
    ]);
}

#[cfg(feature = "glob")]
#[test]
fn test_expand_url_combination() {
    let url = "https://choosealicense.com/licenses/bsd-[2-4]-clause{,-clear}/";

    assert_eq!(expand_url(url), [
        "https://choosealicense.com/licenses/bsd-2-clause/",
        "https://choosealicense.com/licenses/bsd-2-clause-clear/",
        "https://choosealicense.com/licenses/bsd-3-clause/",
        "https://choosealicense.com/licenses/bsd-3-clause-clear/",
        "https://choosealicense.com/licenses/bsd-4-clause/",
        "https://choosealicense.com/licenses/bsd-4-clause-clear/",
    ]);
}