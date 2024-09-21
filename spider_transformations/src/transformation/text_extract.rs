use spider::lazy_static::lazy_static;
use spider::packages::scraper::{ElementRef, Selector};

lazy_static! {
    pub static ref SELECTOR: std::sync::Arc<Selector> =
        unsafe { Selector::parse(&r##"body"##).unwrap_unchecked().into() };
    pub static ref TEXT_SELECTOR: Selector =
        unsafe { Selector::parse(&r##":not(script, style)"##).unwrap_unchecked() };
}

/// extract the text from an element
pub fn extract_text(d: &Vec<ElementRef<'_>>) -> String {
    let mut text = String::new();
    let mut tracker = std::collections::HashSet::new();

    for v in d {
        let t = v
            .children()
            .filter_map(|child| {
                // element should be
                ElementRef::wrap(child)
            })
            .flat_map(|el| {
                let d = el.select(&TEXT_SELECTOR);
                let mut v: Vec<String> = Vec::new();

                let mut it = d.peekable();

                while let Some(ele) = it.next() {
                    let id = ele.id();
                    if !tracker.contains(&id) {
                        tracker.insert(id);
                    }
                    // prevent duplicates
                    let capable = match ele.parent() {
                        Some(p) => {
                            let pid = p.id();
                            if tracker.contains(&pid) {
                                false
                            } else {
                                true
                            }
                        }
                        _ => true,
                    };

                    if capable {
                        let cc = ele
                            .children()
                            .filter_map(|child| {
                                let valid = match ele.parent() {
                                    Some(e) => match e.value().as_element() {
                                        Some(e) => {
                                            let n = e.name();

                                            if n == "script" || n == "style" {
                                                false
                                            } else {
                                                true
                                            }
                                        }
                                        _ => false,
                                    },
                                    _ => true,
                                };
                                if !valid {
                                    None
                                } else {
                                    let ele = ElementRef::wrap(child);
                                    match ele {
                                        Some(e) => {
                                            let n = e.value().name();
                                            if n == "script" || n == "style" {
                                                None
                                            } else {
                                                Some(e)
                                            }
                                        }
                                        _ => None,
                                    }
                                }
                            })
                            .flat_map(|el| el.text())
                            .collect::<Vec<_>>();

                        let push_nl = if cc.len() > 0 {
                            !cc[cc.len() - 1].ends_with("\n")
                        } else {
                            false
                        };

                        // todo: fix whitespace period insert by manual joining or collecting the data. remove replace
                        let mut n = cc.join(" ");
                        let n = if n.ends_with(" .") {
                            n.replace_range(n.len() - 2..n.len(), ".");
                            n
                        } else {
                            n
                        };

                        v.push(n);

                        // peek and make sure the content is not empty space
                        if push_nl && it.peek().is_some() {
                            v.push("\n".into());
                        }
                    }
                }

                v
            })
            .collect::<String>();

        text.push_str(&t.split_whitespace().collect::<Vec<&str>>().join(" "));
    }

    text
}
