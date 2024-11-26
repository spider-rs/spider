use phf::phf_set;
use url::Url;

use crate::page::ONLY_RESOURCES;

static PROTOCOLS: phf::Set<&'static str> = phf_set! {
    "http://",
    "https://",
    "ftp://",
    "ws://"
};

/// Convert to absolute path
#[inline]
pub(crate) fn convert_abs_path(base: &Url, href: &str) -> Url {
    let href = href.trim();

    if href.is_empty() {
        return base.clone();
    }

    if !href.starts_with("/") {
        // Check if href begins with a known protocol
        if href.len() >= 5 && PROTOCOLS.contains(&href[0..5])
            || (href.len() >= 6 && PROTOCOLS.contains(&href[0..6]))
            || (href.len() >= 7 && PROTOCOLS.contains(&href[0..7]))
            || (href.len() >= 8 && PROTOCOLS.contains(&href[0..8]))
        {
            if let Ok(mut next_url) = Url::parse(href) {
                next_url.set_fragment(None);
                return next_url;
            }
        }

        // Handle domain-like href (contains a dot and no spaces) as absolute URLs non assets
        if let Some(position) = href.rfind('.') {
            let hlen = href.len();
            let has_asset = hlen - position;

            if has_asset >= 3 {
                let next_position = position + 1;

                if !ONLY_RESOURCES.contains::<case_insensitive_string::CaseInsensitiveString>(
                    &href[next_position..].into(),
                ) {
                    let full_url = format!("http://{}", href);

                    return Url::parse(&full_url).unwrap_or_else(|_| base.clone());
                }
            }
        }
    }

    let mut base_domain = base.clone();

    if let Ok(mut path) = base_domain.path_segments_mut() {
        path.clear();
    }

    // Use base.join for other relative paths
    match base_domain.join(href) {
        Ok(mut joined) => {
            joined.set_fragment(None);
            joined
        }
        Err(_) => base_domain.clone(),
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
    fn test_join_with_fragment() {
        let base = Url::parse("https://example.com/path/").unwrap();
        let href = "subpage#fragment";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/subpage");
    }

    #[test]
    fn test_join_with_query() {
        let base = Url::parse("https://example.com/path/").unwrap();
        let href = "subpage?query=123";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/subpage?query=123");
    }

    #[test]
    fn test_base_with_fragment() {
        let base = Url::parse("https://example.com/path/#section").unwrap();
        let href = "subpage";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/subpage");
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
    fn test_invalid_href() {
        let base = Url::parse("https://www.example.com/path/").unwrap();
        let href = "other-path/";
        let result = convert_abs_path(&base, href);

        assert_eq!(
            result.as_str(),
            "https://www.example.com/other-path/",
            "Expected base URL for invalid href"
        );
    }

    #[test]
    fn test_domain_like_link() {
        let base = Url::parse("https://www.example.com/path/").unwrap();
        let href = "example.org/another-path";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "http://example.org/another-path",
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
    fn test_bare_relative_path() {
        let base = Url::parse("https://www.example.com/path/").unwrap();
        let href = "another-path";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "https://www.example.com/another-path",
            "Should include path as root-relative when no slash"
        );
    }

    #[test]
    fn test_no_protocol_with_slash() {
        let base = Url::parse("https://www.example.com/path/").unwrap();
        let href = "example.com/other-path";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "http://example.com/other-path",
            "Should treat domain-like href as full URL"
        );
    }
}
