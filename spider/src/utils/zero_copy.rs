//! Zero-copy byte-level parsing for HTTP wire formats and protocol structures.
//!
//! Uses the `zerocopy` crate to reinterpret raw byte slices as typed structs
//! without copying, validated at compile time via derive macros.
//!
//! # Key types
//!
//! - [`HttpStatusLine`] — parse an HTTP/1.x status line directly from a byte buffer
//! - [`ContentTypeMatcher`] — O(1) content-type classification from raw header bytes
//! - [`DnsCacheRecord`] — compact, zerocopy-friendly DNS cache wire format
//! - [`CacheEntryHeader`] — header for cached HTTP responses on disk/mmap

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

// ─── HTTP Status Line ───────────────────────────────────────────────────────

/// Compact representation of an HTTP/1.x status line.
///
/// Can be constructed from raw response bytes without allocating.
/// The version, status code, and reason phrase offsets are stored inline.
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct HttpStatusLine {
    /// HTTP major version (1 for HTTP/1.x).
    pub version_major: u8,
    /// HTTP minor version (0 or 1 for HTTP/1.0 / HTTP/1.1).
    pub version_minor: u8,
    /// The 3-digit status code as a u16 (e.g. 200, 404, 503).
    pub status_code: u16,
}

impl HttpStatusLine {
    /// Parse an HTTP/1.x status line from a byte slice.
    ///
    /// Expects the standard format: `HTTP/1.1 200 OK\r\n`
    /// Returns `None` if the bytes don't match the expected format.
    #[inline]
    pub fn parse(buf: &[u8]) -> Option<Self> {
        // Minimum: "HTTP/1.1 200" = 12 bytes
        if buf.len() < 12 {
            return None;
        }

        // Check "HTTP/" prefix.
        if &buf[..5] != b"HTTP/" {
            return None;
        }

        let version_major = buf[5].wrapping_sub(b'0');
        // buf[6] should be '.'
        if buf[6] != b'.' {
            return None;
        }
        let version_minor = buf[7].wrapping_sub(b'0');

        // buf[8] should be ' '
        if buf[8] != b' ' {
            return None;
        }

        // Parse 3-digit status code.
        let d1 = buf[9].wrapping_sub(b'0') as u16;
        let d2 = buf[10].wrapping_sub(b'0') as u16;
        let d3 = buf[11].wrapping_sub(b'0') as u16;

        if d1 > 9 || d2 > 9 || d3 > 9 {
            return None;
        }

        let status_code = d1 * 100 + d2 * 10 + d3;

        Some(Self {
            version_major,
            version_minor,
            status_code,
        })
    }

    /// Whether this is an informational response (1xx).
    #[inline]
    pub fn is_informational(&self) -> bool {
        self.status_code >= 100 && self.status_code < 200
    }

    /// Whether this is a success response (2xx).
    #[inline]
    pub fn is_success(&self) -> bool {
        self.status_code >= 200 && self.status_code < 300
    }

    /// Whether this is a redirect (3xx).
    #[inline]
    pub fn is_redirect(&self) -> bool {
        self.status_code >= 300 && self.status_code < 400
    }

    /// Whether this is a client error (4xx).
    #[inline]
    pub fn is_client_error(&self) -> bool {
        self.status_code >= 400 && self.status_code < 500
    }

    /// Whether this is a server error (5xx).
    #[inline]
    pub fn is_server_error(&self) -> bool {
        self.status_code >= 500 && self.status_code < 600
    }

    /// Whether this status is retryable (5xx except 501, plus 429, 408).
    #[inline]
    pub fn is_retryable(&self) -> bool {
        match self.status_code {
            429 | 408 => true,
            500 | 502 | 503 | 504 | 507 | 508 | 598 | 599 => true,
            _ => false,
        }
    }
}

// ─── Content-Type Matcher ───────────────────────────────────────────────────

/// Classification of a Content-Type header value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContentTypeClass {
    /// text/html or application/xhtml+xml — should be parsed for links.
    Html = 0,
    /// text/xml, application/xml, etc. — may contain sitemaps or feeds.
    Xml = 1,
    /// application/json — structured data, skip link parsing.
    Json = 2,
    /// text/plain — may contain URLs but no markup.
    PlainText = 3,
    /// text/css, application/javascript — skip entirely.
    WebAsset = 4,
    /// image/*, audio/*, video/* — binary, skip entirely.
    Media = 5,
    /// application/pdf, application/zip, etc. — binary, skip entirely.
    Binary = 6,
    /// Unknown or unrecognised content type.
    Unknown = 7,
}

impl ContentTypeClass {
    /// Whether this content type should be crawled for links.
    #[inline]
    pub fn should_crawl(&self) -> bool {
        matches!(self, Self::Html | Self::Xml)
    }

    /// Whether this content type is binary and should not be streamed/stored.
    #[inline]
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Media | Self::Binary)
    }
}

/// Classify a Content-Type header value directly from raw bytes.
///
/// Operates on the raw header value bytes (e.g. `b"text/html; charset=utf-8"`)
/// without allocating. Handles parameter stripping (`;` delimited) and
/// case-insensitive matching.
///
/// # Performance
/// Single pass, no allocation, early termination on first byte mismatch.
#[inline]
pub fn classify_content_type(raw: &[u8]) -> ContentTypeClass {
    // Strip parameters: take everything before first ';' and trim whitespace.
    // memchr uses SIMD-accelerated byte scanning.
    let mime = match memchr::memchr(b';', raw) {
        Some(pos) => &raw[..pos],
        None => raw,
    };

    // Trim trailing whitespace.
    let mime = trim_ascii_end(mime);

    if mime.is_empty() {
        return ContentTypeClass::Unknown;
    }

    // Fast path: check first byte to branch quickly.
    match mime[0] | 0x20 {
        // ASCII lowercase
        b't' => classify_text(mime),
        b'a' => classify_application(mime),
        b'i' => {
            if starts_with_ignore_case(mime, b"image/") {
                ContentTypeClass::Media
            } else {
                ContentTypeClass::Unknown
            }
        }
        b'v' => {
            if starts_with_ignore_case(mime, b"video/") {
                ContentTypeClass::Media
            } else {
                ContentTypeClass::Unknown
            }
        }
        b'm' => {
            if starts_with_ignore_case(mime, b"multipart/") {
                ContentTypeClass::Unknown
            } else {
                ContentTypeClass::Unknown
            }
        }
        _ => ContentTypeClass::Unknown,
    }
}

#[inline]
fn classify_text(mime: &[u8]) -> ContentTypeClass {
    if starts_with_ignore_case(mime, b"text/html") {
        ContentTypeClass::Html
    } else if starts_with_ignore_case(mime, b"text/xml") {
        ContentTypeClass::Xml
    } else if starts_with_ignore_case(mime, b"text/plain") {
        ContentTypeClass::PlainText
    } else if starts_with_ignore_case(mime, b"text/css")
        || starts_with_ignore_case(mime, b"text/javascript")
    {
        ContentTypeClass::WebAsset
    } else {
        ContentTypeClass::Unknown
    }
}

#[inline]
fn classify_application(mime: &[u8]) -> ContentTypeClass {
    if starts_with_ignore_case(mime, b"application/xhtml+xml")
        || starts_with_ignore_case(mime, b"application/xhtml")
    {
        ContentTypeClass::Html
    } else if starts_with_ignore_case(mime, b"application/xml")
        || starts_with_ignore_case(mime, b"application/rss+xml")
        || starts_with_ignore_case(mime, b"application/atom+xml")
    {
        ContentTypeClass::Xml
    } else if starts_with_ignore_case(mime, b"application/json")
        || starts_with_ignore_case(mime, b"application/ld+json")
    {
        ContentTypeClass::Json
    } else if starts_with_ignore_case(mime, b"application/javascript")
        || starts_with_ignore_case(mime, b"application/wasm")
    {
        ContentTypeClass::WebAsset
    } else if starts_with_ignore_case(mime, b"application/pdf")
        || starts_with_ignore_case(mime, b"application/zip")
        || starts_with_ignore_case(mime, b"application/x-rar")
        || starts_with_ignore_case(mime, b"application/x-tar")
        || starts_with_ignore_case(mime, b"application/x-7z")
        || starts_with_ignore_case(mime, b"application/x-rpm")
        || starts_with_ignore_case(mime, b"application/x-shockwave-flash")
        || starts_with_ignore_case(mime, b"application/octet-stream")
        || starts_with_ignore_case(mime, b"application/vnd.")
    {
        ContentTypeClass::Binary
    } else {
        ContentTypeClass::Unknown
    }
}

/// Case-insensitive byte prefix match.
///
/// Processes 8 bytes at a time using u64 word loads when possible,
/// falling back to byte-at-a-time for remainders.
#[inline]
fn starts_with_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    let len = needle.len();
    if haystack.len() < len {
        return false;
    }
    let h = &haystack[..len];

    // Process 8 bytes at a time via u64 word comparison.
    let chunks = len / 8;
    for i in 0..chunks {
        let off = i * 8;
        let mut hw = u64::from_ne_bytes([
            h[off],
            h[off + 1],
            h[off + 2],
            h[off + 3],
            h[off + 4],
            h[off + 5],
            h[off + 6],
            h[off + 7],
        ]);
        let mut nw = u64::from_ne_bytes([
            needle[off],
            needle[off + 1],
            needle[off + 2],
            needle[off + 3],
            needle[off + 4],
            needle[off + 5],
            needle[off + 6],
            needle[off + 7],
        ]);
        // ASCII lowercase: set bit 5 on every byte.
        hw |= 0x2020_2020_2020_2020;
        nw |= 0x2020_2020_2020_2020;
        if hw != nw {
            return false;
        }
    }

    // Remainder bytes.
    for j in (chunks * 8)..len {
        if h[j] | 0x20 != needle[j] | 0x20 {
            return false;
        }
    }
    true
}

/// Trim trailing ASCII whitespace from a byte slice.
#[inline]
fn trim_ascii_end(s: &[u8]) -> &[u8] {
    let mut end = s.len();
    while end > 0 && s[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &s[..end]
}

// ─── Cache Entry Header ─────────────────────────────────────────────────────

/// On-disk/mmap header for cached HTTP responses.
///
/// This struct is designed to be read directly from a memory-mapped cache file
/// via `zerocopy::Ref::from_prefix()`, eliminating deserialization overhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct CacheEntryHeader {
    /// Magic bytes for validation: `b"SPDR"` = `0x52445053`.
    pub magic: u32,
    /// Length of the URL string that follows this header.
    pub url_len: u32,
    /// Length of the response headers blob that follows the URL.
    pub headers_len: u32,
    /// Length of the response body that follows the headers.
    pub body_len: u32,
    /// Unix timestamp (seconds since epoch) when this entry was cached.
    pub cached_at: u64,
    /// TTL in seconds from `cached_at`.
    pub ttl_secs: u32,
    /// HTTP status code (e.g. 200, 404).
    pub status_code: u16,
    /// Format version (currently 1).
    pub version: u8,
    /// Content-type class (see [`ContentTypeClass`]).
    pub content_type: u8,
}

impl CacheEntryHeader {
    /// Expected magic value: `b"SPDR"`.
    pub const MAGIC: u32 = u32::from_le_bytes(*b"SPDR");

    /// Current format version.
    pub const VERSION: u8 = 1;

    /// Total size of this header in bytes.
    pub const SIZE: usize = std::mem::size_of::<Self>();

    /// Validate that a byte buffer starts with a valid cache entry header.
    #[inline]
    pub fn from_bytes(buf: &[u8]) -> Option<&Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        let (header_ref, _) = zerocopy::Ref::<&[u8], Self>::from_prefix(buf).ok()?;
        let header: &Self = zerocopy::Ref::into_ref(header_ref);
        if header.magic != Self::MAGIC || header.version != Self::VERSION {
            return None;
        }
        Some(header)
    }

    /// Create a new cache entry header.
    pub fn new(
        status_code: u16,
        content_type: ContentTypeClass,
        url_len: u32,
        headers_len: u32,
        body_len: u32,
        ttl_secs: u32,
    ) -> Self {
        Self {
            magic: Self::MAGIC,
            version: Self::VERSION,
            content_type: content_type as u8,
            status_code,
            url_len,
            headers_len,
            body_len,
            cached_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            ttl_secs,
        }
    }

    /// Whether this cache entry has expired.
    #[inline]
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now > self.cached_at + self.ttl_secs as u64
    }

    /// Total size of the full cache entry (header + url + headers + body).
    #[inline]
    pub fn total_entry_size(&self) -> usize {
        Self::SIZE + self.url_len as usize + self.headers_len as usize + self.body_len as usize
    }

    /// Extract the URL slice from a buffer that starts after this header.
    #[inline]
    pub fn url_from<'a>(&self, payload: &'a [u8]) -> Option<&'a [u8]> {
        let len = self.url_len as usize;
        if payload.len() >= len {
            Some(&payload[..len])
        } else {
            None
        }
    }

    /// Extract the body slice from a buffer that starts after this header.
    #[inline]
    pub fn body_from<'a>(&self, payload: &'a [u8]) -> Option<&'a [u8]> {
        let url_end = self.url_len as usize;
        let headers_end = url_end + self.headers_len as usize;
        let body_end = headers_end + self.body_len as usize;
        if payload.len() >= body_end {
            Some(&payload[headers_end..body_end])
        } else {
            None
        }
    }
}

// ─── DNS Cache Record ───────────────────────────────────────────────────────

/// Compact zerocopy-friendly DNS A/AAAA record for cache serialisation.
///
/// Designed for memory-mapped DNS caches where thousands of records need
/// to be scanned without deserialisation overhead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C)]
pub struct DnsCacheRecord {
    /// For A records: IPv4 address (network byte order). For AAAA: all 16 bytes.
    pub addr_bytes: [u8; 16],
    /// TTL in seconds.
    pub ttl_secs: u32,
    /// Hostname length (the hostname string follows this struct).
    pub hostname_len: u16,
    /// Record type: 4 = A (IPv4), 6 = AAAA (IPv6).
    pub addr_type: u8,
    /// Padding for alignment.
    pub _pad: u8,
}

impl DnsCacheRecord {
    /// Total fixed-size portion.
    pub const SIZE: usize = std::mem::size_of::<Self>();

    /// Create from an IPv4 address.
    pub fn from_ipv4(addr: [u8; 4], ttl_secs: u32, hostname_len: u16) -> Self {
        let mut addr_bytes = [0u8; 16];
        addr_bytes[..4].copy_from_slice(&addr);
        Self {
            addr_type: 4,
            addr_bytes,
            ttl_secs,
            hostname_len,
            _pad: 0,
        }
    }

    /// Create from an IPv6 address.
    pub fn from_ipv6(addr: [u8; 16], ttl_secs: u32, hostname_len: u16) -> Self {
        Self {
            addr_type: 6,
            addr_bytes: addr,
            ttl_secs,
            hostname_len,
            _pad: 0,
        }
    }

    /// Read from a byte slice.
    #[inline]
    pub fn from_bytes(buf: &[u8]) -> Option<&Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        let (r, _) = zerocopy::Ref::<&[u8], Self>::from_prefix(buf).ok()?;
        Some(zerocopy::Ref::into_ref(r))
    }

    /// Get the IP address as `std::net::IpAddr`.
    #[inline]
    pub fn to_ip_addr(&self) -> std::net::IpAddr {
        match self.addr_type {
            4 => {
                let mut octets = [0u8; 4];
                octets.copy_from_slice(&self.addr_bytes[..4]);
                std::net::IpAddr::V4(std::net::Ipv4Addr::from(octets))
            }
            _ => std::net::IpAddr::V6(std::net::Ipv6Addr::from(self.addr_bytes)),
        }
    }
}

// ─── Inline header-value extraction ─────────────────────────────────────────

/// Extract Content-Length from raw header bytes without allocation.
///
/// Scans for `Content-Length:` (case-insensitive) and parses the value inline.
#[inline]
pub fn extract_content_length(headers_raw: &[u8]) -> Option<u64> {
    const NEEDLE: &[u8] = b"content-length";

    let mut i = 0;
    while i + NEEDLE.len() + 1 < headers_raw.len() {
        // Find a line that starts with "content-length" (case-insensitive).
        if headers_raw[i] | 0x20 == b'c' && starts_with_ignore_case(&headers_raw[i..], NEEDLE) {
            let after = i + NEEDLE.len();
            // Skip optional whitespace and ':'
            let mut j = after;
            while j < headers_raw.len() && (headers_raw[j] == b' ' || headers_raw[j] == b':') {
                j += 1;
            }
            // Parse digits.
            let mut val: u64 = 0;
            while j < headers_raw.len() && headers_raw[j].is_ascii_digit() {
                val = val
                    .wrapping_mul(10)
                    .wrapping_add((headers_raw[j] - b'0') as u64);
                j += 1;
            }
            if j > after + 1 {
                return Some(val);
            }
        }
        // Advance to next line.
        while i < headers_raw.len() && headers_raw[i] != b'\n' {
            i += 1;
        }
        i += 1; // skip '\n'
    }
    None
}

/// Extract Content-Type value bytes from raw headers without allocation.
///
/// Returns a slice pointing into the original buffer.
#[inline]
pub fn extract_content_type_bytes(headers_raw: &[u8]) -> Option<&[u8]> {
    const NEEDLE: &[u8] = b"content-type";

    let mut i = 0;
    while i + NEEDLE.len() + 1 < headers_raw.len() {
        if headers_raw[i] | 0x20 == b'c' && starts_with_ignore_case(&headers_raw[i..], NEEDLE) {
            let mut j = i + NEEDLE.len();
            // Skip ':' and whitespace.
            while j < headers_raw.len() && (headers_raw[j] == b':' || headers_raw[j] == b' ') {
                j += 1;
            }
            let start = j;
            // Read until CR, LF, or end.
            while j < headers_raw.len() && headers_raw[j] != b'\r' && headers_raw[j] != b'\n' {
                j += 1;
            }
            if j > start {
                return Some(&headers_raw[start..j]);
            }
        }
        while i < headers_raw.len() && headers_raw[i] != b'\n' {
            i += 1;
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── HttpStatusLine tests ───────────────────────────────────────────

    #[test]
    fn test_parse_status_200() {
        let sl = HttpStatusLine::parse(b"HTTP/1.1 200 OK\r\n").unwrap();
        assert_eq!(sl.version_major, 1);
        assert_eq!(sl.version_minor, 1);
        assert_eq!(sl.status_code, 200);
        assert!(sl.is_success());
    }

    #[test]
    fn test_parse_status_404() {
        let sl = HttpStatusLine::parse(b"HTTP/1.1 404 Not Found\r\n").unwrap();
        assert_eq!(sl.status_code, 404);
        assert!(sl.is_client_error());
    }

    #[test]
    fn test_parse_status_503() {
        let sl = HttpStatusLine::parse(b"HTTP/1.1 503 Service Unavailable\r\n").unwrap();
        assert_eq!(sl.status_code, 503);
        assert!(sl.is_server_error());
        assert!(sl.is_retryable());
    }

    #[test]
    fn test_parse_http10() {
        let sl = HttpStatusLine::parse(b"HTTP/1.0 301 Moved Permanently\r\n").unwrap();
        assert_eq!(sl.version_major, 1);
        assert_eq!(sl.version_minor, 0);
        assert_eq!(sl.status_code, 301);
        assert!(sl.is_redirect());
    }

    #[test]
    fn test_parse_minimal() {
        let sl = HttpStatusLine::parse(b"HTTP/1.1 200").unwrap();
        assert_eq!(sl.status_code, 200);
    }

    #[test]
    fn test_parse_too_short() {
        assert!(HttpStatusLine::parse(b"HTTP/1.1 20").is_none());
    }

    #[test]
    fn test_parse_bad_prefix() {
        assert!(HttpStatusLine::parse(b"HTTZ/1.1 200 OK\r\n").is_none());
    }

    #[test]
    fn test_parse_bad_version_sep() {
        assert!(HttpStatusLine::parse(b"HTTP/1X1 200 OK\r\n").is_none());
    }

    #[test]
    fn test_status_429_retryable() {
        let sl = HttpStatusLine::parse(b"HTTP/1.1 429 Too Many Requests\r\n").unwrap();
        assert!(sl.is_retryable());
    }

    #[test]
    fn test_status_201_not_retryable() {
        let sl = HttpStatusLine::parse(b"HTTP/1.1 201 Created\r\n").unwrap();
        assert!(!sl.is_retryable());
    }

    #[test]
    fn test_informational() {
        let sl = HttpStatusLine::parse(b"HTTP/1.1 100 Continue\r\n").unwrap();
        assert!(sl.is_informational());
    }

    // ─── ContentType classification tests ───────────────────────────────

    #[test]
    fn test_classify_html() {
        assert_eq!(
            classify_content_type(b"text/html; charset=utf-8"),
            ContentTypeClass::Html
        );
        assert_eq!(classify_content_type(b"TEXT/HTML"), ContentTypeClass::Html);
    }

    #[test]
    fn test_classify_xhtml() {
        assert_eq!(
            classify_content_type(b"application/xhtml+xml"),
            ContentTypeClass::Html
        );
    }

    #[test]
    fn test_classify_xml() {
        assert_eq!(
            classify_content_type(b"application/xml"),
            ContentTypeClass::Xml
        );
        assert_eq!(classify_content_type(b"text/xml"), ContentTypeClass::Xml);
        assert_eq!(
            classify_content_type(b"application/rss+xml"),
            ContentTypeClass::Xml
        );
    }

    #[test]
    fn test_classify_json() {
        assert_eq!(
            classify_content_type(b"application/json"),
            ContentTypeClass::Json
        );
        assert_eq!(
            classify_content_type(b"application/ld+json"),
            ContentTypeClass::Json
        );
    }

    #[test]
    fn test_classify_media() {
        assert_eq!(
            classify_content_type(b"image/jpeg"),
            ContentTypeClass::Media
        );
        assert_eq!(classify_content_type(b"video/mp4"), ContentTypeClass::Media);
        assert_eq!(classify_content_type(b"image/png"), ContentTypeClass::Media);
    }

    #[test]
    fn test_classify_binary() {
        assert_eq!(
            classify_content_type(b"application/pdf"),
            ContentTypeClass::Binary
        );
        assert_eq!(
            classify_content_type(b"application/zip"),
            ContentTypeClass::Binary
        );
        assert_eq!(
            classify_content_type(b"application/octet-stream"),
            ContentTypeClass::Binary
        );
    }

    #[test]
    fn test_classify_web_assets() {
        assert_eq!(
            classify_content_type(b"text/css"),
            ContentTypeClass::WebAsset
        );
        assert_eq!(
            classify_content_type(b"application/javascript"),
            ContentTypeClass::WebAsset
        );
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(classify_content_type(b""), ContentTypeClass::Unknown);
        assert_eq!(
            classify_content_type(b"something/weird"),
            ContentTypeClass::Unknown
        );
    }

    #[test]
    fn test_classify_with_params() {
        assert_eq!(
            classify_content_type(b"text/html; charset=utf-8; boundary=something"),
            ContentTypeClass::Html
        );
    }

    #[test]
    fn test_should_crawl() {
        assert!(ContentTypeClass::Html.should_crawl());
        assert!(ContentTypeClass::Xml.should_crawl());
        assert!(!ContentTypeClass::Json.should_crawl());
        assert!(!ContentTypeClass::Binary.should_crawl());
    }

    #[test]
    fn test_is_binary() {
        assert!(ContentTypeClass::Media.is_binary());
        assert!(ContentTypeClass::Binary.is_binary());
        assert!(!ContentTypeClass::Html.is_binary());
    }

    // ─── CacheEntryHeader tests ─────────────────────────────────────────

    #[test]
    fn test_cache_entry_header_roundtrip() {
        let header = CacheEntryHeader::new(200, ContentTypeClass::Html, 25, 100, 5000, 3600);

        // Serialize to bytes.
        let bytes = zerocopy::IntoBytes::as_bytes(&header);
        assert_eq!(bytes.len(), CacheEntryHeader::SIZE);

        // Deserialize from bytes.
        let parsed = CacheEntryHeader::from_bytes(bytes).unwrap();
        assert_eq!(parsed.status_code, 200);
        assert_eq!(parsed.content_type, ContentTypeClass::Html as u8);
        assert_eq!(parsed.url_len, 25);
        assert_eq!(parsed.headers_len, 100);
        assert_eq!(parsed.body_len, 5000);
        assert_eq!(parsed.ttl_secs, 3600);
    }

    #[test]
    fn test_cache_entry_header_bad_magic() {
        let mut bytes = [0u8; CacheEntryHeader::SIZE];
        bytes[0..4].copy_from_slice(b"XXXX");
        assert!(CacheEntryHeader::from_bytes(&bytes).is_none());
    }

    #[test]
    fn test_cache_entry_header_too_short() {
        assert!(CacheEntryHeader::from_bytes(&[0u8; 4]).is_none());
    }

    #[test]
    fn test_cache_entry_total_size() {
        let header = CacheEntryHeader::new(200, ContentTypeClass::Html, 20, 50, 1000, 60);
        assert_eq!(
            header.total_entry_size(),
            CacheEntryHeader::SIZE + 20 + 50 + 1000
        );
    }

    #[test]
    fn test_cache_entry_body_extraction() {
        let header = CacheEntryHeader::new(200, ContentTypeClass::Html, 3, 2, 5, 60);
        // payload = url(3) + headers(2) + body(5)
        let payload = b"abcdeHELLO";
        let body = header.body_from(payload).unwrap();
        assert_eq!(body, b"HELLO");
    }

    #[test]
    fn test_cache_entry_url_extraction() {
        let header = CacheEntryHeader::new(200, ContentTypeClass::Html, 5, 0, 0, 60);
        let payload = b"hello";
        let url = header.url_from(payload).unwrap();
        assert_eq!(url, b"hello");
    }

    // ─── DnsCacheRecord tests ───────────────────────────────────────────

    #[test]
    fn test_dns_record_ipv4() {
        let rec = DnsCacheRecord::from_ipv4([192, 168, 1, 1], 300, 11);
        assert_eq!(rec.addr_type, 4);
        let ip = rec.to_ip_addr();
        assert_eq!(
            ip,
            std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 1))
        );
    }

    #[test]
    fn test_dns_record_ipv6() {
        let addr: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let rec = DnsCacheRecord::from_ipv6(addr, 600, 9);
        assert_eq!(rec.addr_type, 6);
        let ip = rec.to_ip_addr();
        assert_eq!(ip, std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn test_dns_record_roundtrip() {
        let rec = DnsCacheRecord::from_ipv4([10, 0, 0, 1], 120, 7);
        let bytes = zerocopy::IntoBytes::as_bytes(&rec);
        let parsed = DnsCacheRecord::from_bytes(bytes).unwrap();
        assert_eq!(parsed.addr_type, 4);
        assert_eq!(parsed.ttl_secs, 120);
        assert_eq!(parsed.hostname_len, 7);
    }

    // ─── Inline header extraction tests ─────────────────────────────────

    #[test]
    fn test_extract_content_length() {
        let raw = b"Host: example.com\r\nContent-Length: 12345\r\nConnection: keep-alive\r\n";
        assert_eq!(extract_content_length(raw), Some(12345));
    }

    #[test]
    fn test_extract_content_length_case_insensitive() {
        let raw = b"content-length: 999\r\n";
        assert_eq!(extract_content_length(raw), Some(999));
    }

    #[test]
    fn test_extract_content_length_missing() {
        let raw = b"Host: example.com\r\n";
        assert_eq!(extract_content_length(raw), None);
    }

    #[test]
    fn test_extract_content_type_bytes() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\nHost: x\r\n";
        let ct = extract_content_type_bytes(raw).unwrap();
        assert_eq!(ct, b"text/html; charset=utf-8");
    }

    #[test]
    fn test_extract_content_type_missing() {
        let raw = b"Host: example.com\r\n";
        assert!(extract_content_type_bytes(raw).is_none());
    }

    // ─── HttpStatusLine zerocopy layout test ────────────────────────────

    #[test]
    fn test_status_line_size() {
        assert_eq!(std::mem::size_of::<HttpStatusLine>(), 4);
    }

    #[test]
    fn test_cache_header_size_stable() {
        // repr(C) layout: u32 u32 u32 u32 u64 u32 u16 u8 u8 = 32 bytes
        assert_eq!(CacheEntryHeader::SIZE, 32);
    }

    #[test]
    fn test_dns_record_size_stable() {
        // repr(C) layout: [u8;16] u32 u16 u8 u8 = 24 bytes
        assert_eq!(DnsCacheRecord::SIZE, 24);
    }

    // ─── Edge cases ─────────────────────────────────────────────────────

    #[test]
    fn test_classify_content_type_trailing_space() {
        assert_eq!(
            classify_content_type(b"text/html   "),
            ContentTypeClass::Html
        );
    }

    #[test]
    fn test_classify_vnd_prefix() {
        assert_eq!(
            classify_content_type(b"application/vnd.ms-excel"),
            ContentTypeClass::Binary
        );
    }

    #[test]
    fn test_status_line_as_bytes() {
        let sl = HttpStatusLine {
            version_major: 1,
            version_minor: 1,
            status_code: 200,
        };
        let bytes = zerocopy::IntoBytes::as_bytes(&sl);
        assert_eq!(bytes.len(), 4);
        // Can be read back.
        let parsed: HttpStatusLine = zerocopy::FromBytes::read_from_bytes(bytes).unwrap();
        assert_eq!(sl.status_code, parsed.status_code);
    }
}
