use phf::phf_set;
use url::Url;

/// Acceptable protocols for data transfering TCP.
static PROTOCOLS: phf::Set<&'static str> = phf_set! {
    "http://",
    "https://",
    "ftp://",
    "ws://"
};

/// Ignore protocols for web crawling.
static IGNORED_PROTOCOLS: phf::Set<&'static str> = phf_set! {
    "file:",
    "sms:",
    "javascript:",
    "data:",
    "whatsapp:",
    "geo:",
    "skype:",
    "ssh:",
    "zoommtg:",
    "market:",
    "intent:",
    "mailto:",
    "tel:",
};

/// convert abs path url
pub(crate) fn convert_abs_url(u: &mut Url) {
    if let Ok(mut path) = u.path_segments_mut() {
        path.clear();
    }
}

/// Parse the absolute url
pub fn parse_absolute_url(url: &str) -> Option<Box<Url>> {
    match url::Url::parse(url) {
        Ok(mut u) => {
            convert_abs_url(&mut u);
            u.set_query(None);
            Some(Box::new(u))
        }
        _ => None,
    }
}

/// Firewall protection. This does nothing without the [firewall] flag.
#[cfg(feature = "firewall")]
pub(crate) fn block_website(u: &Url) -> bool {
    let mut blocked = false;

    if let Some(host) = u.host_str() {
        if spider_firewall::is_bad_website_url(host) {
            blocked = true;
        }
    }

    blocked
}

/// Firewall protection. This does nothing without the [firewall] flag.
#[cfg(not(feature = "firewall"))]
pub(crate) fn block_website(_u: &Url) -> bool {
    false
}

/// Return handling for the links
enum LinkReturn {
    /// Early return
    EarlyReturn,
    /// Empty ignore
    Empty,
    /// Absolute url
    Absolute(Url),
}

#[inline]
/// Handle the base url return to determine 3rd party urls.
fn handle_base(href: &str) -> LinkReturn {
    if href.is_empty() || href == "#" || href == "javascript:void(0);" {
        return LinkReturn::EarlyReturn;
    }

    // handle absolute urls.
    if !href.starts_with("/") {
        // ignore protocols that are not crawlable
        if let Some(protocol_end) = href.find(':') {
            // Extract the potential protocol up to and including the ':'
            let protocol_slice_section = &href[..protocol_end + 1];

            // Ignore protocols that are in the IGNORED_PROTOCOLS set
            if IGNORED_PROTOCOLS.contains(protocol_slice_section) {
                return LinkReturn::EarlyReturn;
            }

            // protocol_end is the byte position of ':' (ASCII). PROTOCOLS
            // entries are ASCII, but the BYTES that follow `:` in `href`
            // can be the middle of a multi-byte UTF-8 codepoint when the
            // input is a non-ASCII URL (e.g. `ab:éé` where the second `é`
            // straddles the byte that `proto_end` points to). The previous
            // direct-slice form (`&href[..proto_end]`) panicked on those
            // links — production reproduced
            // `panicked at spider-2.51.154/src/utils/abs.rs:101` on
            // tokio-rt-worker. Use `get(..)` so a non-char-boundary slice
            // returns `None` instead of unwinding the worker thread.
            let proto_end = protocol_end + 3;
            if let Some(prefix) = href.get(..proto_end) {
                if PROTOCOLS.contains(prefix) {
                    if let Ok(mut next_url) = Url::parse(href) {
                        next_url.set_fragment(None);
                        return LinkReturn::Absolute(next_url);
                    }
                }
            }
        }
    }

    LinkReturn::Empty
}

/// Convert to absolute path. The base url must be the root path to avoid infinite appending.
/// We always handle the urls from the base path.
#[inline]
pub fn convert_abs_path(base: &Url, href: &str) -> Url {
    let href = href.trim();

    if base.as_str() == href {
        return base.to_owned();
    }

    match handle_base(href) {
        LinkReturn::Absolute(u) => return u,
        LinkReturn::EarlyReturn => return base.to_owned(),
        _ => (),
    }

    // we can swap the domains if they do not match incase of crawler redirect anti-bot
    match base.join(href) {
        Ok(mut joined) => {
            joined.set_fragment(None);
            joined
        }
        Err(_) => base.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{convert_abs_path, handle_base, LinkReturn};
    use crate::utils::parse_absolute_url;

    /// Regression: `handle_base` byte-sliced `&href[..proto_end]` where
    /// `proto_end = colon_byte + 3`. When the bytes immediately after
    /// `:` were the middle of a multi-byte UTF-8 codepoint (e.g.
    /// `ab:éé` — second `é` straddles byte 5, the slice end), the slice
    /// op panicked — reproduced production-side as
    /// `panicked at spider-2.51.154/src/utils/abs.rs:101` on tokio-rt-
    /// workers. Using `href.get(..proto_end)` falls through with `None`
    /// instead. This test pins each shape that previously panicked.
    #[test]
    fn handle_base_does_not_panic_on_non_ascii_after_colon() {
        // Each of these would have panicked under the old slice form.
        for href in [
            "ab:éé",      // colon at byte 2 → proto_end=5 → mid-`é`
            "x:éy",       // colon at byte 1 → proto_end=4 → mid-`é`
            "abc:中文",   // colon at byte 3 → proto_end=6 → mid-`中`
            "ab:🦀de",    // 🦀 = 4-byte → mid-codepoint slice
            "ab:éé/path", // similar pattern with trailing path
        ] {
            // The function must return without unwinding. We only care
            // that it does not panic; whatever shape it returns is fine.
            let _ = handle_base(href);
        }
    }

    /// Real protocol prefixes still match through the new `get(..)`
    /// path — protect against accidentally widening the early-return
    /// when fixing the panic.
    #[test]
    fn handle_base_still_recognizes_http_https_protocols() {
        for url in [
            "https://example.com/path",
            "http://example.org/",
            "https://example.com/path?q=v#frag",
        ] {
            assert!(
                matches!(handle_base(url), LinkReturn::Absolute(_)),
                "expected Absolute for {url}"
            );
        }
    }

    #[test]
    fn test_basic_join() {
        let base = parse_absolute_url("https://example.com/path/").unwrap();
        let href = "/subpage";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/subpage");
    }

    #[test]
    fn test_absolute_href() {
        let base = parse_absolute_url("https://example.com/path/").unwrap();
        let href = "https://example.org/anotherpath";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), href);
    }

    #[test]
    fn test_slash_join() {
        let base = parse_absolute_url("https://example.com/path/").unwrap();
        let href = "/absolute";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/absolute");
    }

    #[test]
    fn test_empty_href() {
        let base = parse_absolute_url("https://example.com/path/").unwrap();
        let href = "";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/");
    }

    #[test]
    fn test_double_dot_href() {
        let base = parse_absolute_url("https://example.com/path/").unwrap();
        let href = "..";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/");
    }

    #[test]
    fn test_domain_like_link() {
        let base = parse_absolute_url("https://www.example.com/path/").unwrap();
        let href = "example.org/another-path";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "https://www.example.com/example.org/another-path",
            "Should treat as a domain"
        );
    }

    #[test]
    fn test_relative_path_with_slash() {
        let base = parse_absolute_url("https://www.example.com/path/").unwrap();
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
        let base = parse_absolute_url("https://www.example.com/path/").unwrap();
        let href = "example.com/other-path";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "https://www.example.com/example.com/other-path",
            "Should treat domain-like href as full URL"
        );
    }

    #[test]
    fn test_no_invalid_protocols() {
        let base = parse_absolute_url("https://www.example.com").unwrap();
        let href = "mailto:info@laminarpharma.com";
        let result = convert_abs_path(&base, href);

        assert_eq!(
            result.as_str(),
            "https://www.example.com/",
            "Should treat domain-like href as full URL"
        );
    }

    #[test]
    fn test_convert_abs_path_query_string() {
        let base = parse_absolute_url("https://example.com").unwrap();
        let href = "/page?key=value&other=123";
        let result = convert_abs_path(&base, href);
        assert_eq!(
            result.as_str(),
            "https://example.com/page?key=value&other=123"
        );
    }

    #[test]
    fn test_convert_abs_path_fragment() {
        let base = parse_absolute_url("https://example.com").unwrap();
        // Fragments are stripped
        let href = "/page#section";
        let result = convert_abs_path(&base, href);
        assert_eq!(result.as_str(), "https://example.com/page");
    }

    #[test]
    fn test_convert_abs_path_encoded_url() {
        let base = parse_absolute_url("https://example.com").unwrap();
        let href = "/path%20with%20spaces";
        let result = convert_abs_path(&base, href);
        assert!(result.as_str().contains("path%20with%20spaces"));
    }

    #[test]
    fn test_convert_abs_path_port_number() {
        let base = parse_absolute_url("https://example.com:8080").unwrap();
        let href = "/api/data";
        let result = convert_abs_path(&base, href);
        assert!(result.as_str().contains(":8080"));
        assert!(result.as_str().contains("/api/data"));
    }

    #[test]
    fn test_convert_abs_path_deep_relative() {
        let base = parse_absolute_url("https://example.com/a/b/c/").unwrap();
        let href = "../../d";
        let result = convert_abs_path(&base, href);
        // base is stripped to root by parse_absolute_url, so join resolves from root
        assert!(result.as_str().starts_with("https://example.com"));
    }
}
