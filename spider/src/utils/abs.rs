use crate::page::ONLY_RESOURCES;
use phf::phf_set;
use url::Url;

static PROTOCOLS: phf::Set<&'static str> = phf_set! {
    "http://",
    "https://",
    "ftp://",
    "ws://"
};

/// convert abs path url
pub(crate) fn convert_abs_url(u: &mut Url) {
    if let Ok(mut path) = u.path_segments_mut() {
        path.clear();
    }
}

/// convert abs path without url returning a new url
pub(crate) fn convert_abs_url_base(u: &Url) -> Url {
    let mut u = u.clone();
    convert_abs_url(&mut u);
    u
}

/// Parse the absolute url
pub(crate) fn parse_absolute_url(url: &str) -> Option<Box<Url>> {
    match url::Url::parse(url) {
        Ok(mut u) => {
            convert_abs_url(&mut u);
            Some(Box::new(u))
        }
        _ => None,
    }
}

/// Convert to absolute path. The base url must be the root path to avoid infinite appending.
#[inline]
pub(crate) fn convert_abs_path(base: &Url, href: &str) -> Url {
    let href = href.trim();

    if href.is_empty() {
        return base.clone();
    }

    // handle absolute urls.
    if !href.starts_with("/") {
        let length = href.len();

        let protocol_slice = if length >= 8 && href.is_char_boundary(8) {
            &href[0..8]
        } else if length >= 7 && href.is_char_boundary(7) {
            &href[0..7]
        } else if length >= 6 && href.is_char_boundary(6) {
            &href[0..6]
        } else if length >= 5 && href.is_char_boundary(5) {
            &href[0..5]
        } else {
            ""
        };

        if let Some(protocol_end) = protocol_slice.find("://") {
            let protocol_slice = &href[..protocol_end + 3]; // +3 to include "://"

            if PROTOCOLS.contains(protocol_slice) {
                if let Ok(mut next_url) = Url::parse(href) {
                    next_url.set_fragment(None);
                    return next_url;
                }
            }
        }

        if let Some(position) = href.rfind('.') {
            let hlen = href.len();
            let has_asset = hlen - position;
            if has_asset >= 3 {
                let next_position = position + 1;
                if !ONLY_RESOURCES.contains::<case_insensitive_string::CaseInsensitiveString>(
                    &href[next_position..].into(),
                ) {
                    let full_url = format!("{}://{}", base.scheme(), href);
                    if let Ok(mut next_url) = Url::parse(&full_url) {
                        next_url.set_fragment(None);
                        return next_url;
                    }
                }
            }
        }
    }

    // we can swap the domains if they do not match incase of crawler redirect anti-bot
    match base.join(href) {
        Ok(mut joined) => {
            joined.set_fragment(None);
            joined
        }
        Err(_) => base.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::convert_abs_path;
    use url::Url;

    #[test]
    fn test_basic_join() {
        let base = Url::parse("https://example.com/path/").unwrap();
        let href = "/subpage";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/subpage");
    }

    #[test]
    fn test_absolute_href() {
        let base = Url::parse("https://example.com/path/").unwrap();
        let href = "https://example.org/anotherpath";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), href);
    }

    #[test]
    fn test_slash_join() {
        let base = Url::parse("https://example.com/path/").unwrap();
        let href = "/absolute";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/absolute");
    }

    #[test]
    fn test_empty_href() {
        let base = Url::parse("https://example.com/path/").unwrap();
        let href = "";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/path/");
    }

    #[test]
    fn test_double_dot_href() {
        let base = Url::parse("https://example.com/path/").unwrap();
        let href = "..";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/");
    }

    #[test]
    fn test_domain_like_link() {
        let base = Url::parse("https://www.example.com/path/").unwrap();
        let href = "example.org/another-path";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "https://example.org/another-path",
            "Should treat as a domain"
        );
    }

    #[test]
    fn test_relative_path_with_slash() {
        let base = Url::parse("https://www.example.com/path/").unwrap();
        let href = "/another-path";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "https://www.example.com/another-path",
            "Should correctly join relative path with leading slash"
        );
    }

    #[test]
    fn test_no_protocol_with_slash() {
        let base = Url::parse("https://www.example.com/path/").unwrap();
        let href = "example.com/other-path";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "https://example.com/other-path",
            "Should treat domain-like href as full URL"
        );
    }
}
