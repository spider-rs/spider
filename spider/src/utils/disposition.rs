//! What a fetched page turned out to be: the site's content, an anti-bot
//! challenge, a block, or no usable document.
//!
//! Challenge and block pages often arrive with status 200, so the status alone
//! can't say. This looks only for strong, vendor-specific markers, so an
//! ordinary page that merely embeds a CAPTCHA widget stays `Content`.

/// What a fetched page turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PageDisposition {
    /// The page's own content.
    Content,
    /// An anti-bot interstitial a visitor must pass first: Anubis, Cloudflare's
    /// "Just a moment…", DDoS-Guard, Sucuri, Imperva and the like.
    Challenge,
    /// The site refused the request: 401, 403, 429 or a known deny page.
    Blocked,
    /// No usable document: a 5xx (including spider's own failure codes), a
    /// browser error page, or an empty body.
    Error,
}

impl PageDisposition {
    /// Whether the page holds the site's content, so it is safe to index.
    pub fn is_content(self) -> bool {
        self == PageDisposition::Content
    }
}

/// Challenge pages are small; markers that could appear in an ordinary page
/// only count below this size.
const SMALL_PAGE: usize = 64 * 1024;

/// Markers specific enough to mean a challenge at any page size.
const CHALLENGE_MARKERS: &[&str] = &[
    "id=\"anubis_challenge\"",        // Anubis
    "window._cf_chl_opt",             // Cloudflare managed challenge
    "/cdn-cgi/challenge-platform/h/", // Cloudflare challenge script
    "sucuri_cloudproxy_js",           // Sucuri
];

/// Markers that mean a challenge only on a small page.
const SMALL_PAGE_CHALLENGE_MARKERS: &[&str] = &[
    "<title>just a moment...</title>",                  // Cloudflare
    "<title>making sure you're not a bot!</title>",     // Anubis
    "<title>making sure you&#39;re not a bot!</title>", // Anubis, escaped
    "_incapsula_resource",                              // Imperva
    "<title>ddos-guard</title>",                        // DDoS-Guard
    "captcha-delivery.com",                             // DataDome
];

/// Deny pages that come back with a 200.
const BLOCK_MARKERS: &[&str] = &[
    "<title>oh noes!</title>", // Anubis deny
    "<title>access denied | ", // Cloudflare 1020
    "<h1>access denied</h1>",  // Akamai
];

/// Classify a fetched page from its status, headers, body and final URL.
pub fn classify_page(
    status: u16,
    headers: Option<&reqwest::header::HeaderMap>,
    body: &[u8],
    final_url: &str,
) -> PageDisposition {
    if final_url.starts_with("chrome-error://") || final_url.starts_with("about:blank") {
        return PageDisposition::Error;
    }
    let mitigated = headers
        .and_then(|h| h.get("cf-mitigated"))
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("challenge"));
    if mitigated {
        return PageDisposition::Challenge;
    }

    let head = &body[..body.len().min(SMALL_PAGE)];
    let lower = String::from_utf8_lossy(head).to_ascii_lowercase();
    let has = |marker: &str| lower.contains(&marker.to_ascii_lowercase());
    // A deny page can reuse the challenge template (Anubis's error page keeps
    // an empty `anubis_challenge` block), so its markers are checked first.
    if body.len() <= SMALL_PAGE && BLOCK_MARKERS.iter().any(|m| has(m)) {
        return PageDisposition::Blocked;
    }
    if CHALLENGE_MARKERS.iter().any(|m| has(m))
        || (body.len() <= SMALL_PAGE && SMALL_PAGE_CHALLENGE_MARKERS.iter().any(|m| has(m)))
    {
        return PageDisposition::Challenge;
    }
    // After the markers: Cloudflare serves its challenge with a 403.
    if matches!(status, 401 | 403 | 429) {
        return PageDisposition::Blocked;
    }
    if status >= 500 || body.iter().all(|b| b.is_ascii_whitespace()) {
        return PageDisposition::Error;
    }
    PageDisposition::Content
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://example.com/";

    #[test]
    fn ordinary_pages_are_content() {
        let page = b"<html><head><title>Docs</title></head><body>Hello</body></html>";
        assert_eq!(
            classify_page(200, None, page, URL),
            PageDisposition::Content
        );
        // A page that embeds a CAPTCHA widget in a form is still content.
        let form = br#"<html><body><form><div class="g-recaptcha" data-sitekey="x"></div></form><script src="https://www.google.com/recaptcha/api.js"></script></body></html>"#;
        assert_eq!(
            classify_page(200, None, form, URL),
            PageDisposition::Content
        );
        assert_eq!(
            classify_page(404, None, b"<html><body>Not found</body></html>", URL),
            PageDisposition::Content
        );
    }

    #[test]
    fn challenge_pages_are_recognised_whatever_the_status() {
        let anubis = br#"<html><head><title>Making sure you&#39;re not a bot!</title><script id="anubis_challenge" type="application/json">{}</script></head></html>"#;
        assert_eq!(
            classify_page(200, None, anubis, URL),
            PageDisposition::Challenge
        );
        let cloudflare = br#"<html><head><title>Just a moment...</title></head><body><script>window._cf_chl_opt={}</script></body></html>"#;
        assert_eq!(
            classify_page(403, None, cloudflare, URL),
            PageDisposition::Challenge
        );
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("cf-mitigated", "challenge".parse().unwrap());
        assert_eq!(
            classify_page(200, Some(&headers), b"<html></html>", URL),
            PageDisposition::Challenge
        );
    }

    #[test]
    fn a_small_page_marker_does_not_flag_a_large_page() {
        let mut big = b"<html><head><title>Just a moment...</title></head><body>".to_vec();
        big.extend(std::iter::repeat(b'x').take(SMALL_PAGE + 1));
        assert_eq!(
            classify_page(200, None, &big, URL),
            PageDisposition::Content
        );
    }

    #[test]
    fn blocks_and_errors() {
        // Anubis's error page keeps an empty challenge block; it is a denial.
        let anubis_denied = br#"<html><head><title>Oh noes!</title><script id="anubis_challenge" type="application/json">null</script></head></html>"#;
        assert_eq!(
            classify_page(200, None, anubis_denied, URL),
            PageDisposition::Blocked
        );
        assert_eq!(
            classify_page(403, None, b"<html><body>Forbidden</body></html>", URL),
            PageDisposition::Blocked
        );
        assert_eq!(
            classify_page(429, None, b"slow down", URL),
            PageDisposition::Blocked
        );
        let denied = b"<html><head><title>Oh noes!</title></head></html>";
        assert_eq!(
            classify_page(200, None, denied, URL),
            PageDisposition::Blocked
        );
        assert_eq!(
            classify_page(503, None, b"<html></html>", URL),
            PageDisposition::Error
        );
        assert_eq!(classify_page(525, None, b"", URL), PageDisposition::Error);
        assert_eq!(
            classify_page(200, None, b"  \n", URL),
            PageDisposition::Error
        );
        assert_eq!(
            classify_page(200, None, b"<html></html>", "chrome-error://chromewebdata/"),
            PageDisposition::Error
        );
    }
}
