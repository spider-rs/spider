use crate::utils::abs::convert_abs_path;
use crate::utils::templates::EMPTY_HTML_BASIC;
#[cfg(not(feature = "decentralized"))]
use crate::utils::RequestError;
use crate::utils::{
    css_selectors::{compiled_base_element_selector, compiled_selector, compiled_xml_selector},
    get_domain_from_url, hash_html, networking_capable, BasicCachePolicy, CacheOptions,
    PageResponse,
};
use crate::CaseInsensitiveString;
use crate::Client;
use crate::RelativeSelectors;
use crate::{compact_str::CompactString, utils::templates::EMPTY_HTML};
use auto_encoder::auto_encode_bytes;
use hashbrown::HashSet;
use lol_html::AsciiCompatibleEncoding;
use phf::phf_set;
use regex::bytes::Regex;
use reqwest::StatusCode;
use std::sync::Arc;
use tokio::time::Duration;
#[cfg(feature = "time")]
use tokio::time::Instant;

#[cfg(all(feature = "decentralized", feature = "headers"))]
use crate::utils::FetchPageResult;
use lazy_static::lazy_static;
use url::Url;

/// Construct an element content handler tuple using a pre-compiled `&'static Selector`.
/// Same as `lol_html::element!` but uses `Cow::Borrowed` to avoid re-parsing.
macro_rules! element_precompiled {
    ($selector:expr, $handler:expr) => {{
        #[inline(always)]
        const fn type_hint<'h, T, H: lol_html::HandlerTypes>(h: T) -> T
        where
            T: FnMut(&mut lol_html::html_content::Element<'_, '_, H>) -> lol_html::HandlerResult
                + 'h,
        {
            h
        }
        (
            std::borrow::Cow::Borrowed($selector),
            lol_html::send::ElementContentHandlers::default().element(type_hint($handler)),
        )
    }};
}

/// Allocate up to 128kb upfront for small pages.
pub(crate) const MAX_PRE_ALLOCATED_HTML_PAGE_SIZE: u64 = 128 * 1024;
/// Allocate up to 128kb upfront for small pages.
pub(crate) const MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE: usize =
    MAX_PRE_ALLOCATED_HTML_PAGE_SIZE as usize;

/// Maximum bytes to pre-allocate based on Content-Length (10 MB).
/// Prevents a malicious server from triggering instant OOM by sending
/// `Content-Length: 2147483648` with a tiny body. Vec grows naturally
/// beyond this via doubling, so large legitimate responses still work.
pub(crate) const MAX_PREALLOC: usize = 10 * 1024 * 1024;

/// Reject responses whose Content-Length exceeds this hard ceiling (2 GB).
/// Avoids even starting to stream a response that cannot fit in memory.
pub(crate) const MAX_CONTENT_LENGTH: u64 = 2 * 1024 * 1024 * 1024;

/// Allocate up to 16kb * 4 upfront for small pages.
#[cfg(feature = "chrome")]
pub(crate) const TURNSTILE_WALL_PAGE_SIZE: usize = MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE * 4;

lazy_static! {
    /// Wildcard match all domains.
    static ref CASELESS_WILD_CARD: CaseInsensitiveString = CaseInsensitiveString::new("*");
    static ref SSG_CAPTURE: Regex =  Regex::new(r#""(.*?)""#).unwrap();
    /// Gatsby
    static ref GATSBY: Option<&'static str> =  Some("gatsby-chunk-mapping");
    /// Nuxt.
    static ref NUXT_DATA: Option<&'static str> =  Some("__NUXT_DATA__");
    /// Nuxt.
    static ref NUXT: Option<&'static str> =  Some("__nuxt");
    /// React ssr app.
    static ref REACT_SSR: Option<&'static str> =  Some("react-app.embeddedData");
    /// Unknown status (generic fallback)
    pub(crate) static ref UNKNOWN_STATUS_ERROR: StatusCode =
        StatusCode::from_u16(599).expect("valid status code");
    /// Chrome-style timeout / network unknown
    pub(crate) static ref CHROME_UNKNOWN_STATUS_ERROR: StatusCode =
        StatusCode::from_u16(598).expect("valid status code");
    /// Connection timeout
    pub(crate) static ref CONNECTION_TIMEOUT_ERROR: StatusCode =
        StatusCode::from_u16(524).expect("valid status code");
    /// Connection refused (origin down)
    pub(crate) static ref CONNECTION_REFUSED_ERROR: StatusCode =
        StatusCode::from_u16(521).expect("valid status code");
    /// Connection aborted
    pub(crate) static ref CONNECTION_ABORTED_ERROR: StatusCode =
        StatusCode::from_u16(522).expect("valid status code");
    /// Connection reset
    pub(crate) static ref CONNECTION_RESET_ERROR: StatusCode =
        StatusCode::from_u16(523).expect("valid status code");
    /// DNS failure
    pub(crate) static ref DNS_RESOLVE_ERROR: StatusCode =
        StatusCode::from_u16(525).expect("valid status code");
    /// Address / origin permanently unreachable — the destination host or port
    /// cannot be reached from this path, and no amount of proxy rotation or
    /// retrying is going to change that. Emitted for:
    ///   - io::ErrorKind::{HostUnreachable, NetworkUnreachable}
    ///   - Chrome `net::ERR_ADDRESS_UNREACHABLE` (e.g. SOCKS reply 0x04)
    ///   - Chrome `net::ERR_CONNECTION_REFUSED` (origin port not listening)
    ///   - SSL/TLS handshake failures (cipher / protocol version mismatch,
    ///     invalid cert chain) — both reqwest and Chrome paths. These cannot
    ///     be fixed by retrying with the same client, so they are bucketed
    ///     with reachable-but-refused.
    /// Kept distinct from 525 (DNS) so operators can tell DNS-dead from
    /// reachable-but-refused targets; both are excluded from retry.
    pub(crate) static ref ADDRESS_UNREACHABLE_ERROR: StatusCode =
        StatusCode::from_u16(526).expect("valid status code");
    /// Redirect-chain cap exceeded (IANA 310 "Loop Detected" — 3xx keeps retry
    /// strategies from treating this as a transient 5xx).
    pub(crate) static ref TOO_MANY_REDIRECTS_ERROR: StatusCode =
        StatusCode::from_u16(310).expect("valid status code");
    /// Body decode failure
    pub(crate) static ref BODY_DECODE_ERROR: StatusCode =
        StatusCode::from_u16(400).expect("valid status code");
    /// Request malformed or unreachable
    pub(crate) static ref UNREACHABLE_REQUEST_ERROR: StatusCode =
        StatusCode::from_u16(503).expect("valid status code");
}

lazy_static! {
    /// Aho-Corasick automaton for DNS error detection — single O(n) scan.
    ///
    /// Safety net for resolvers whose errors do not surface through the
    /// typed `io::Error(NotFound)` fast path. Patterns cover:
    /// - tokio / system getaddrinfo → "dns error" / "failed to lookup address"
    /// - glibc                      → "Name or service not known"
    /// - macOS / BSD NODATA         → "No address associated with hostname"
    /// - Node-style runtimes        → "ENOTFOUND"
    /// - hickory ResolveError       → "no record found"
    /// - our DnsCacheResolver       → "dns resolution returned no addresses"
    static ref DNS_ERROR_AC: aho_corasick::AhoCorasick = aho_corasick::AhoCorasick::new([
        "dns error",
        "failed to lookup address",
        "Name or service not known",
        "No address associated with hostname",
        "ENOTFOUND",
        "no record found",
        "dns resolution returned no addresses",
    ]).expect("valid patterns");

    /// Aho-Corasick automaton for SSL/TLS handshake error detection — single
    /// O(n) scan against an `err.to_string()`. The destination's TLS protocol
    /// or cipher suite is incompatible with the client; retrying through a
    /// different proxy will not change that.
    ///
    /// Patterns are kept deliberately specific to error-message phrasing to
    /// avoid false positives from URLs or response bodies that happen to
    /// contain TLS-adjacent words (e.g. a URL like `/tls-handshake-demo` would
    /// NOT match because reqwest URL-encodes `?` and spaces, and the surrounding
    /// punctuation in the patterns below is unlikely in any encoded URL).
    /// Patterns cover both rustls and native-tls (OpenSSL) error surfaces, plus
    /// Chrome `net::ERR_*` strings that may surface through wrapped HTTP paths:
    /// - rustls         → "received fatal alert: HandshakeFailure",
    ///                    "alert: HandshakeFailure",
    ///                    "alert: ProtocolVersion"
    /// - native-tls     → "no shared cipher", "wrong_version_number",
    ///                    "wrong version number", "unsupported protocol"
    /// - generic / wrap → "tls handshake error", "TLS handshake error",
    ///                    "tls handshake failure", "TLS handshake failure",
    ///                    "tls handshake failed", "TLS handshake failed",
    ///                    "ssl handshake failure", "SSL handshake failure",
    ///                    "ssl handshake failed",  "SSL handshake failed"
    /// - Chrome surface → "ERR_SSL_VERSION_OR_CIPHER_MISMATCH",
    ///                    "ERR_SSL_PROTOCOL_ERROR"
    static ref SSL_HANDSHAKE_ERROR_AC: aho_corasick::AhoCorasick = aho_corasick::AhoCorasick::new([
        // rustls — colon disambiguates from incidental URL hits
        "alert: HandshakeFailure",
        "alert: ProtocolVersion",
        "received fatal alert",
        // OpenSSL / native-tls
        "no shared cipher",
        "wrong_version_number",
        "wrong version number",
        "unsupported protocol",
        // generic / wrap — punctuation+verb tightens the match
        "tls handshake error",
        "TLS handshake error",
        "tls handshake failure",
        "TLS handshake failure",
        "tls handshake failed",
        "TLS handshake failed",
        "ssl handshake failure",
        "SSL handshake failure",
        "ssl handshake failed",
        "SSL handshake failed",
        // Chrome `net::ERR_*` strings — uppercase+underscore, URL-safe
        "ERR_SSL_VERSION_OR_CIPHER_MISMATCH",
        "ERR_SSL_PROTOCOL_ERROR",
    ]).expect("valid patterns");
}

/// Check if a connect error is a DNS resolution failure.
/// Zero-alloc fast path via `downcast_ref`, single-alloc fallback via Aho-Corasick O(n) scan.
/// Only called when `err.is_connect()` is already true.
fn is_dns_error(err: &crate::client::Error) -> bool {
    use std::error::Error;

    // Fast path: walk source chain looking for io::Error with specific kinds.
    // Hickory DNS surfaces as: reqwest → hyper → io::Error(Custom("dns error: ..."))
    let mut source: Option<&(dyn Error + 'static)> = err.source();
    let mut depth = 0u8;
    while let Some(e) = source {
        if depth >= 6 {
            break;
        }
        if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
            if matches!(io_err.kind(), std::io::ErrorKind::NotFound) {
                return true;
            }
        }
        source = e.source();
        depth += 1;
    }

    // Slow fallback: single to_string() + single-pass Aho-Corasick scan.
    DNS_ERROR_AC.is_match(&err.to_string())
}

/// Check whether a reqwest error is a permanent SSL/TLS handshake failure.
///
/// Fast path: gated on `err.is_connect() || err.is_request()` — reqwest only
/// emits SSL errors through these two classifiers, so any other kind (timeout,
/// body decode, status, redirect) returns instantly without allocating the
/// Display string. This keeps the success path zero-cost (the helper isn't
/// even reached when the request succeeded) and the non-SSL error path
/// allocation-free.
///
/// Slow path: a single Aho-Corasick pass over `err.to_string()` (one alloc)
/// covering rustls, native-tls, generic-wrap and Chrome-wrapped surfaces.
/// Patterns are tightened to phrases that include punctuation/verbs so they
/// cannot accidentally match URL-encoded query strings.
fn is_ssl_handshake_error(err: &crate::client::Error) -> bool {
    if !(err.is_connect() || err.is_request()) {
        return false;
    }
    SSL_HANDSHAKE_ERROR_AC.is_match(&err.to_string())
}

/// Whether a status code is retryable (transient server/network errors).
/// DNS errors (525), address-unreachable / SSL-handshake (526), and
/// redirect-cap hits (310) are permanent and excluded. Three 5xx codes are
/// explicitly excluded too because the server is announcing a deterministic
/// "won't do" condition that no retry can change:
/// - 501 Not Implemented (server doesn't recognise the method)
/// - 505 HTTP Version Not Supported (HTTP version mismatch)
/// - 511 Network Authentication Required (captive portal — needs login)
#[inline]
pub fn is_retryable_status(status: StatusCode) -> bool {
    status != *DNS_RESOLVE_ERROR
        && status != *ADDRESS_UNREACHABLE_ERROR
        && status != *TOO_MANY_REDIRECTS_ERROR
        && status != StatusCode::NOT_IMPLEMENTED
        && status != StatusCode::HTTP_VERSION_NOT_SUPPORTED
        && status != StatusCode::NETWORK_AUTHENTICATION_REQUIRED
        && (status.is_server_error()
            || matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT
            ))
}

/// Get the HTTP status code of errors.

pub(crate) fn get_error_http_status_code(err: &crate::client::Error) -> StatusCode {
    use std::error::Error;
    use std::io;

    if let Some(status) = err.status() {
        return status;
    }

    if err.is_timeout() {
        return *CONNECTION_TIMEOUT_ERROR;
    }

    // SSL/TLS handshake failures are permanent for this destination — the
    // server's protocol/cipher suite is incompatible with the client. Retrying
    // through another proxy/connection won't fix it. Checked before the
    // is_connect / is_request branches because reqwest classifies SSL errors
    // inconsistently across its TLS backends. Maps to 526 so retry paths skip.
    if is_ssl_handshake_error(err) {
        return *ADDRESS_UNREACHABLE_ERROR;
    }

    // HTTP/2 permanent reasons (`INADEQUATE_SECURITY`, `HTTP_1_1_REQUIRED`).
    // Same "client can't reach this destination" semantics as SSL — bucketed
    // at 526 so `is_retryable_status` returns false. Walks the error chain
    // once via the same extractor used by `should_attempt_retry`.
    #[cfg(not(feature = "decentralized"))]
    if let Some(status) = h2_permanent_reason_status(err) {
        return status;
    }

    if err.is_connect() {
        // DNS resolution failure — never retried
        if is_dns_error(err) {
            return *DNS_RESOLVE_ERROR;
        }
        if let Some(io_err) = err.source().and_then(|e| e.downcast_ref::<io::Error>()) {
            match io_err.kind() {
                io::ErrorKind::ConnectionRefused => return *CONNECTION_REFUSED_ERROR,
                io::ErrorKind::ConnectionAborted => return *CONNECTION_ABORTED_ERROR,
                io::ErrorKind::ConnectionReset => return *CONNECTION_RESET_ERROR,
                io::ErrorKind::NotFound => return *UNREACHABLE_REQUEST_ERROR,
                // Kernel-level "no route" / "host unreachable" means the
                // destination itself is not reachable from this path — no
                // retry through a different proxy will change that. Map
                // to 526 (ADDRESS_UNREACHABLE_ERROR) so `is_retryable_status`
                // treats it as permanent.
                io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => {
                    return *ADDRESS_UNREACHABLE_ERROR
                }
                io::ErrorKind::TimedOut => return *CONNECTION_TIMEOUT_ERROR,
                _ => (),
            }
        }
        return *UNREACHABLE_REQUEST_ERROR;
    }

    if err.is_body() {
        return *BODY_DECODE_ERROR;
    }

    if err.is_request() {
        return StatusCode::BAD_REQUEST;
    }

    *UNKNOWN_STATUS_ERROR
}

/// Check if a script src URL is a known ad, tracker, or analytics script.
/// Uses `spider_network_blocker` trie for absolute URLs and `ADBLOCK_PATTERNS`
/// substring matching for relative paths. Returns `true` if the script should
/// be excluded from upgrade scoring (i.e., it is a tracker/ad, not app logic).
#[cfg(all(not(feature = "decentralized"), feature = "smart"))]
#[inline]
fn is_tracker_script(src: &str) -> bool {
    use chromiumoxide::spider_network_blocker;
    if src.starts_with("http") {
        // Absolute URL — trie prefix match against known tracker domains.
        spider_network_blocker::scripts::URL_IGNORE_TRIE.contains_prefix(src)
    } else {
        // Relative path — substring match against adblock patterns.
        spider_network_blocker::adblock::ADBLOCK_PATTERNS
            .iter()
            .any(|p| src.contains(p))
    }
}

#[cfg(all(not(feature = "decentralized"), feature = "smart"))]
lazy_static! {
    /// General no script patterns.
    static ref NO_SCRIPT_JS_REQUIRED: aho_corasick::AhoCorasick = {
        let patterns = &[
            // JS-required / SPA shell markers
            "enable javascript", "requires javascript", "turn on javascript",
        ];
        aho_corasick::AhoCorasick::new(patterns).expect("valid dom script  patterns")
    };

    /// Methods that cause the dom to mutate.
    static ref DOM_SCRIPT_WATCH_METHODS: aho_corasick::AhoCorasick = {
        let patterns = &[
            ".createElementNS", ".removeChild", ".insertBefore", ".createElement(",
            ".createTextNode", ".replaceChildren(", ".prepend(",
            ".appendChild(", "document.write(", "window.location.href",
            // DOM mutation hot paths
            ".innerHTML", ".outerHTML", ".insertAdjacentHTML(", ".insertAdjacentElement(",
            ".replaceWith(", ".replaceChild(", ".cloneNode(",
            "new DOMParser",
            // SPA routing
            "history.pushState", "history.replaceState",
            "location.assign(", "location.replace(",
            "window.location=", "document.location=",
            // Fetching required
            "fetch(", "new XMLHttpRequest",
            // APPS
            "window.__NUXT__"
        ];
        aho_corasick::AhoCorasick::new(patterns).expect("valid dom script patterns")
    };

    /// Attributes for JS requirements.
    static ref DOM_WATCH_ATTRIBUTE_PATTERNS: [&'static str; 5] = [
        "__NEXT_DATA__", "__NUXT__", "data-reactroot",
        "ng-version", "data-v-app",
    ];

    /// Hydration required
    pub(crate) static ref HYDRATION_IDS: phf::Set<&'static str> = phf_set! {
        "__nuxt",
        "__nuxt-loader",
        "__NUXT_DATA__",
        "__next",
        "__NEXT_DATA__",
        "___gatsby",
        "redwood-app",
        "sapper"
    };
}

lazy_static! {
    /// Downloadable media types [https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/MIME_types/Common_types].
    pub(crate) static ref DOWNLOADABLE_MEDIA_TYPES: phf::Set<&'static str> = phf_set! {
        // --- Audio ---
        "audio/mpeg",        // mp3
        "audio/wav",         // wav
        "audio/ogg",         // ogg/oga/opus
        "audio/flac",        // flac
        "audio/aac",         // aac
        "audio/webm",        // weba
        "audio/midi",        // mid/midi
        "audio/x-midi",      // mid/midi (common)
        "audio/mp4",         // m4a (common)
        "audio/x-m4a",       // m4a (common)
        "audio/aiff",        // aiff (common)
        "audio/x-aiff",      // aiff (common)
        "audio/3gpp",        // 3gp (audio-only)
        "audio/3gpp2",       // 3g2 (audio-only)
        // --- Video ---
        "video/mp4",         // mp4/m4v
        "video/webm",        // webm
        "video/ogg",         // ogv
        "video/x-matroska",  // mkv
        "video/x-msvideo",   // avi
        "video/quicktime",   // mov
        "video/x-ms-wmv",    // wmv
        "video/x-flv",       // flv
        "video/mpeg",        // mpeg
        "video/mp2t",        // ts
        "video/3gpp",        // 3gp (video)
        "video/3gpp2",       // 3g2 (video)
        // --- Images ---
        "image/jpeg",        // jpg/jpeg
        "image/png",         // png
        "image/gif",         // gif
        "image/webp",        // webp
        "image/svg+xml",     // svg
        "image/bmp",         // bmp
        "image/tiff",        // tif/tiff
        "image/vnd.microsoft.icon", // ico
        "image/apng",        // apng
        "image/avif",        // avif
        "image/heic",        // heic (common)
        "image/heif",        // heif (common)
        // --- Fonts ---
        "font/woff",         // woff
        "font/woff2",        // woff2
        "font/ttf",          // ttf
        "font/otf",          // otf
        "application/vnd.ms-fontobject", // eot
        // --- Documents / ebooks ---
        "application/pdf",   // pdf
        "application/rtf",   // rtf
        "text/plain",        // txt
        "text/csv",          // csv
        "text/markdown",     // md
        "text/calendar",     // ics
        "application/msword", // doc
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document", // docx
        "application/vnd.ms-powerpoint", // ppt
        "application/vnd.openxmlformats-officedocument.presentationml.presentation", // pptx
        "application/vnd.ms-excel", // xls
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", // xlsx
        "application/vnd.oasis.opendocument.text", // odt
        "application/vnd.oasis.opendocument.spreadsheet", // ods
        "application/vnd.oasis.opendocument.presentation", // odp
        "application/vnd.visio", // vsd
        "application/epub+zip", // epub
        "application/vnd.amazon.ebook", // azw
        "application/x-abiword", // abw
        // --- Archives / binaries / installers ---
        "application/octet-stream", // generic binary downloads
        "application/zip",          // zip
        "application/x-zip-compressed", // zip (common non-standard)
        "application/vnd.rar",      // rar
        "application/x-rar-compressed", // rar (common)
        "application/x-7z-compressed", // 7z
        "application/x-tar",        // tar
        "application/gzip",         // gz
        "application/x-gzip",       // gz (common)
        "application/x-bzip",       // bz
        "application/x-bzip2",      // bz2
        "application/java-archive", // jar
        "application/x-freearc",    // arc
        "application/vnd.apple.installer+xml", // mpkg
        // --- Ogg container ---
        "application/ogg",          // ogx
    };

    // IGNORE_ASSETS moved to a compile-time phf::Set (IGNORE_EXTENSIONS) below.

    /// The chunk size for the rewriter. Can be adjusted using the env var "SPIDER_STREAMING_CHUNK_SIZE".
    pub(crate) static ref STREAMING_CHUNK_SIZE: usize = {
        let default_streaming_chunk_size: usize = (8192 * num_cpus::get_physical().min(64)).min(65536);
        let min_streaming_chunk_size: usize = default_streaming_chunk_size * 2 / 3;

        std::env::var("SPIDER_STREAMING_CHUNK_SIZE")
            .ok()
            .and_then(|val| val.parse::<usize>().ok())
            .map(|val| {
                if val < min_streaming_chunk_size {
                    min_streaming_chunk_size
                } else {
                    val
                }
            })
            .unwrap_or(default_streaming_chunk_size)
    };
}

/// Compile-time perfect-hash set of ignored asset extensions (all lowercase, no dot prefix).
/// Replaces the former `IGNORE_ASSETS` `HashSet<CaseInsensitiveString>` — zero allocation
/// at init and at lookup time.
pub(crate) static IGNORE_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    // Image
    "jpg", "jpeg", "png", "gif", "svg", "webp", "bmp", "tiff", "tif",
    "heic", "heif", "apng", "avif", "ico",
    // Video
    "mp4", "avi", "mov", "wmv", "flv", "mkv", "webm", "m4v", "mpeg",
    "3gp", "3g2",
    // Audio
    "mp3", "wav", "ogg", "aac", "flac", "m4a", "aiff", "cda", "mid",
    "midi", "oga", "opus", "weba",
    // Font
    "woff", "woff2", "ttf", "otf", "eot",
    // Document
    "pdf", "eps", "rtf", "txt", "doc", "docx", "csv", "epub",
    "abw", "azw", "odt", "ods", "odp", "ppt", "pptx", "xls", "xlsx", "vsd",
    // Data / config
    "yaml", "yml", "ics", "md", "webmanifest",
    // Archive / binary
    "gz", "arc", "bin", "bz", "bz2", "jar", "mpkg", "rar", "tar", "zip", "7z",
    // Legacy plugin
    "swf", "xap",
    // Ogg containers
    "ogv", "ogx",
    // Misc media
    "ts",
};

/// Check whether `ext` (the substring *after* the last dot) is a known asset extension.
/// Zero-allocation: lowercases into a stack buffer and looks up in the compile-time phf set.
#[inline]
pub(crate) fn is_ignored_extension(ext: &str) -> bool {
    let bytes = ext.as_bytes();
    // Longest known extension is "webmanifest" (11 bytes).
    // Anything longer cannot match; empty cannot match either.
    if bytes.len() > 16 || bytes.is_empty() {
        return false;
    }
    let mut buf = [0u8; 16];
    let dest = &mut buf[..bytes.len()];
    dest.copy_from_slice(bytes);
    dest.make_ascii_lowercase();
    // ASCII lowercasing preserves UTF-8 validity; debug_assert guards the invariant.
    debug_assert!(std::str::from_utf8(dest).is_ok());
    let lowered = std::str::from_utf8(dest).unwrap_or_default();
    IGNORE_EXTENSIONS.contains(lowered)
}

/// Global EMA of links-per-page. Used to pre-size extraction HashSets and
/// avoid repeated rehashing on link-dense sites. Lock-free, race-safe:
/// worst case a slightly stale hint, which is still better than a static 32.
static LINK_CAPACITY_HINT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(32);

/// Read the current link capacity hint (minimum 32).
#[inline(always)]
pub(crate) fn link_set_capacity() -> usize {
    LINK_CAPACITY_HINT
        .load(std::sync::atomic::Ordering::Relaxed)
        .max(32)
}

/// Update the EMA after a page extraction. Uses 3:1 weighting (75% old, 25% new).
#[inline(always)]
pub(crate) fn update_link_capacity_hint(count: usize) {
    let prev = LINK_CAPACITY_HINT.load(std::sync::atomic::Ordering::Relaxed);
    let next = if prev == 0 {
        count.max(32)
    } else {
        ((prev * 3 + count) / 4).max(32)
    };
    LINK_CAPACITY_HINT.store(next, std::sync::atomic::Ordering::Relaxed);
}

// Thread-local reusable buffer for XML sitemap parsing.
// Uses Cell take/replace pattern — safe across `.await` points because the
// Vec is taken out (owned) before any await and put back after.
// If the task migrates threads between take and replace, the buffer simply
// lands on a different thread — harmless. Re-entrant calls see an empty Vec
// (no panic, just no reuse for that nested invocation).
thread_local! {
    static XML_PARSE_BUF: std::cell::Cell<Vec<u8>> = const { std::cell::Cell::new(Vec::new()) };
}

/// Byte threshold above which rewriter loops yield to the async runtime.
/// Pages smaller than this are processed without yielding (zero overhead).
pub const REWRITER_YIELD_THRESHOLD: usize = 512 * 1024;

/// How many chunks between yield points for large pages.
pub const REWRITER_YIELD_INTERVAL: usize = 8;

/// The AI data returned from a GPT.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AIResults {
    /// The prompt used for the GPT.
    pub input: String,
    /// The js output of the GPT response.
    pub js_output: String,
    /// The content output returned from the GPT response that is not a browser script, example: extracted data from the markup.
    pub content_output: Vec<String>,
    /// The image of the page.
    pub screenshot_output: Option<Vec<u8>>,
    /// The error that occured if any.
    pub error: Option<String>,
}

/// Results from automation operations (extraction, observation, etc.).
///
/// This struct stores the output from remote multimodal automation
/// and can be used with both Chrome and HTTP-only crawls.
#[cfg(feature = "serde")]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AutomationResults {
    /// The prompt used for the GPT/LLM.
    pub input: String,
    /// The content output returned from the automation.
    pub content_output: serde_json::Value,
    /// The base64 image of the page (if captured).
    pub screenshot_output: Option<String>,
    /// The error that occurred if any.
    pub error: Option<String>,
    /// Token usage for this automation result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::features::automation::AutomationUsage>,
    /// Whether the page is relevant to crawl goals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevant: Option<bool>,
    /// Number of automation steps executed, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps_executed: Option<usize>,
    /// Optional reasoning text returned by the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

/// Results from automation operations (extraction, observation, etc.).
#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Default)]
pub struct AutomationResults {
    /// The prompt used for the GPT/LLM.
    pub input: String,
    /// The content output returned from the automation (as string without serde).
    pub content_output: String,
    /// The base64 image of the page (if captured).
    pub screenshot_output: Option<String>,
    /// The error that occurred if any.
    pub error: Option<String>,
    /// Token usage for this automation result.
    pub usage: Option<crate::features::automation::AutomationUsage>,
    /// Whether the page is relevant to crawl goals.
    pub relevant: Option<bool>,
    /// Number of automation steps executed, when available.
    pub steps_executed: Option<usize>,
    /// Optional reasoning text returned by the model.
    pub reasoning: Option<String>,
}

/// Page-level metadata extracted from HTML.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Metadata {
    /// The `<title>` text from the page.
    pub title: Option<CompactString>,
    /// The `<meta name="description">` content.
    pub description: Option<CompactString>,
    /// The Open Graph image URL (`og:image`).
    pub image: Option<CompactString>,
    #[cfg(feature = "chrome")]
    /// The web automation metadata:
    pub automation: Option<Vec<AutomationResults>>, // /// Optional Open Graph metadata (`<meta property="og:*">`) extracted from the page.
                                                    // #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
                                                    // pub og: Option<Box<OpenGraph>>,
}

impl Metadata {
    /// Does metadata exist?
    pub fn exist(&self) -> bool {
        self.title.is_some() || self.description.is_some() || self.image.is_some()
    }
}

// /// Open Graph metadata extracted from `<meta property="og:*">` tags.
// #[derive(Debug, Default, Clone)]
// #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
// pub struct OpenGraph {
//     /// The Open Graph title (`og:title`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub title: Option<CompactString>,
//     /// The Open Graph description (`og:description`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub description: Option<CompactString>,
//     /// The Open Graph image URL (`og:image`).
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub image: Option<CompactString>,
//     /// The canonical page URL (`og:url`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub url: Option<CompactString>,
//     /// The content type (`og:type`, e.g., "article", "website"). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub content_type: Option<CompactString>,
//     /// The site name (`og:site_name`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub site_name: Option<CompactString>,
//     /// The locale of the content (`og:locale`, e.g., "en_US"). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub locale: Option<CompactString>,
//     /// The author's name (`article:author` or `og:author`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub author: Option<CompactString>,
//     /// The time the content was first published (`article:published_time`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub published_time: Option<CompactString>,
//     /// The time the content was last modified (`article:modified_time`). NOT USED.
//     #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
//     pub modified_time: Option<CompactString>,
// }

/// Enumeration of known anti-bot and fraud prevention technologies.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AntiBotTech {
    /// Cloudflare Bot Management - integrated with CDN/WAF, provides behavioral and ML detection.
    Cloudflare,
    /// DataDome - bot protection focused on e-commerce, travel, and classifieds.
    DataDome,
    /// HUMAN (formerly White Ops) - advanced bot mitigation with a focus on ad fraud and Satori threat intelligence.
    HUMAN,
    /// PerimeterX - offers Bot Defender and Code Defender with a strong focus on e-commerce and credential stuffing.
    PerimeterX,
    /// Kasada - bot defense using client-side interrogation and cryptographic challenges.
    Kasada,
    /// FingerprintJS - device fingerprinting and fraud detection via browser and device signal intelligence.
    FingerprintJS,
    /// Arkose Labs - bot mitigation through interactive challenges and risk scoring, used by large enterprises.
    ArkoseLabs,
    /// Imperva - offers bot management as part of a broader WAF/CDN suite with threat intelligence integration.
    Imperva,
    /// F5 - legacy enterprise security vendor offering bot protection through traffic inspection and WAF.
    F5,
    /// Queue-it - virtual waiting room technology to manage surges in traffic and prevent bot scalping.
    QueueIt,
    /// Netacea - behavioral analysis and intent-based detection via server-side interception.
    Netacea,
    /// AppsFlyer - primarily a mobile attribution platform with anti-fraud capabilities.
    AppsFlyer,
    /// Adjust - mobile measurement and analytics provider with fraud prevention modules.
    Adjust,
    /// AppTrana - Indusface is a leading application security SaaS company that secures critical Web, Mobil
    AppTrana,
    /// Akamai Bot Manager - enterprise-grade bot management as part of Akamai’s edge security stack.
    AkamaiBotManager,
    /// Radware Bot Manager - bot detection via intent-based algorithms and real-time profiling.
    RadwareBotManager,
    /// Reblaze - cloud-based web security suite including bot management, WAF, and DDoS mitigation.
    Reblaze,
    /// CHEQ - marketing security and fraud prevention focused on ad spend protection and invalid traffic.
    CHEQ,
    /// Incode - identity verification and fraud prevention with biometric and document validation.
    Incode,
    /// Singula - AI-based marketing and user protection, offering user behavior analysis and fraud insights.
    Singula,
    /// Alibaba TMD (TMall Defense) - anti-bot system used by Alibaba group sites (Taobao, Tmall, Lazada, Miravia, AliExpress).
    AlibabaTMD,
    /// Sucuri Website Firewall - cloud-based WAF and DDoS protection.
    Sucuri,
    /// DDoS-Guard - DDoS protection and CDN service.
    DDoSGuard,
    /// Vercel Firewall - edge protection and security checkpoint for Vercel-hosted sites.
    Vercel,
    /// AWS WAF - Amazon Web Services Web Application Firewall, often paired with CloudFront.
    AwsWaf,
    /// Wordfence - WordPress security plugin with WAF and bot blocking.
    Wordfence,
    /// GeeTest - CAPTCHA provider using slide, click, and behavioral challenges.
    GeeTest,
    /// hCaptcha - privacy-focused CAPTCHA service used as reCAPTCHA alternative.
    HCaptcha,
    /// Custom user-supplied antibot pattern matched (no specific provider identified).
    Custom,
    /// Fallback value if none match or detection failed.
    #[default]
    None,
}

/// Inner state for a spool file, protected by `Arc` so that clones (e.g.
/// from broadcast channels) share the same file without copying it.
/// The file is deleted only when the *last* reference drops.
///
/// When the spool was written inside a [`crate::utils::html_spool::WEBSITE_SPOOL_DIR`]
/// scope, `dir_handle` holds an `Arc` to that per-website directory so
/// it stays alive as long as any page referencing a file inside it is
/// live — guaranteeing that readers keep working even if the owning
/// `Website` was dropped mid-broadcast.  The handle is `None` for files
/// written outside a scope (ad-hoc `page.spool_html_to_disk()` calls
/// against the global dir) so there is zero per-page cost in the
/// non-website path.
#[cfg(feature = "balance")]
#[derive(Debug)]
struct SpoolInner {
    path: std::path::PathBuf,
    /// Extends the lifetime of the per-website spool directory for as
    /// long as this guard is alive.  Unread by design — the `Drop` of
    /// the surrounding `Arc<SpoolInner>` decrements the dir's `Arc`
    /// count, which is the entire contract.
    #[allow(dead_code)]
    dir_handle: Option<Arc<crate::utils::html_spool::WebsiteSpoolDir>>,
}

#[cfg(feature = "balance")]
impl Drop for SpoolInner {
    #[inline]
    fn drop(&mut self) {
        crate::utils::html_spool::track_page_unspooled();
        let path = std::mem::take(&mut self.path);
        if !path.as_os_str().is_empty() {
            // Non-blocking channel send — the dedicated cleanup thread
            // handles the actual file deletion in the background.
            crate::utils::html_spool::queue_spool_delete(path);
        }
    }
}

/// RAII guard for a temporary HTML spool file on disk.
///
/// Internally reference-counted via `Arc` — cloning a guard is a cheap
/// pointer bump (no file copy, no I/O).  The spool file is deleted when
/// the last clone drops, making this safe for broadcast channels where
/// one page may be cloned for N subscribers.
#[cfg(feature = "balance")]
#[derive(Debug, Clone, Default)]
pub(crate) struct HtmlSpoolGuard {
    inner: Option<Arc<SpoolInner>>,
}

#[cfg(feature = "balance")]
impl HtmlSpoolGuard {
    /// Build a guard for a spool file.  When `dir_handle` is `Some`, the
    /// guard also keeps the per-website spool directory alive so readers
    /// that outlive the owning `Website` (e.g. broadcast subscribers) can
    /// still open the file.  When `None`, the file lives in the process-
    /// shared global dir and is cleaned individually on last drop —
    /// identical to the pre-website-scope behaviour, zero extra cost.
    pub fn new(
        path: std::path::PathBuf,
        dir_handle: Option<Arc<crate::utils::html_spool::WebsiteSpoolDir>>,
    ) -> Self {
        Self {
            inner: Some(Arc::new(SpoolInner { path, dir_handle })),
        }
    }

    #[inline]
    pub fn path(&self) -> Option<&std::path::Path> {
        self.inner.as_ref().map(|s| s.path.as_path())
    }
}

/// Represent a page visited.
#[derive(Debug, Clone, Default)]
#[cfg(not(feature = "decentralized"))]
pub struct Page {
    /// The bytes of the resource.
    pub(crate) html: Option<bytes::Bytes>,
    /// Base absolute url for page.
    pub(crate) base: Option<Url>,
    /// The raw url for the page. Useful since Url::parse adds a trailing slash.
    pub(crate) url: String,
    /// The headers of the page request response.
    pub headers: Option<reqwest::header::HeaderMap>,
    #[cfg(feature = "remote_addr")]
    /// The remote address of the page.
    pub remote_addr: Option<core::net::SocketAddr>,
    #[cfg(feature = "cookies")]
    /// The cookies of the page request response.
    pub cookies: Option<reqwest::header::HeaderMap>,
    /// The status code of the page request.
    pub status_code: StatusCode,
    #[cfg(not(feature = "page_error_status_details"))]
    /// The error of the request if any.
    pub error_status: Option<String>,
    #[cfg(feature = "page_error_status_details")]
    /// The error of the request if any.
    pub error_status: Option<std::sync::Arc<reqwest::Error>>,
    /// The external urls to group with the domain
    pub external_domains_caseless: Arc<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Not implemented in the chrome feature].
    pub final_redirect_destination: Option<String>,
    #[cfg(feature = "time")]
    /// The duration from start of parsing to end of gathering links.
    duration: Option<Instant>,
    #[cfg(feature = "chrome")]
    /// Page object for chrome. The page may be closed when accessing it on another thread from concurrency.
    chrome_page: Option<chromiumoxide::Page>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<AIResults>>,
    #[cfg(feature = "gemini")]
    /// The credits used from Gemini in order.
    pub gemini_credits_used: Option<Vec<crate::features::gemini_common::GeminiUsage>>,
    #[cfg(feature = "gemini")]
    /// The extra data from the Gemini AI.
    pub extra_gemini_data: Option<Vec<AIResults>>,
    /// The usage from remote multimodal automation (extraction, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub remote_multimodal_usage: Option<Vec<crate::features::automation::AutomationUsage>>,
    /// The extra data from the remote multimodal automation (extraction results, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub extra_remote_multimodal_data: Option<Vec<AutomationResults>>,
    /// URLs requested by automation to spawn as additional pages.
    pub spawn_pages: Option<Vec<String>>,
    /// Additional content keyed by return format (e.g. `"markdown"`, `"text"`).
    /// Populated when multiple formats are requested via
    /// [`SpiderCloudConfig::with_return_formats`](crate::configuration::SpiderCloudConfig::with_return_formats).
    #[cfg(feature = "spider_cloud")]
    pub content_map: Option<hashbrown::HashMap<String, bytes::Bytes>>,
    /// The links found on the page. This includes all links that have an href url.
    pub page_links: Option<Box<HashSet<CaseInsensitiveString>>>,
    /// The request should retry.
    pub should_retry: bool,
    /// A WAF was found on the page.
    pub waf_check: bool,
    /// The total byte transferred for the page. Mainly used for chrome events. Inspect the content for bytes when using http instead.
    pub bytes_transferred: Option<f64>,
    /// The page was blocked from crawling usual from using website::on_should_crawl_callback.
    pub blocked_crawl: bool,
    /// The signature of the page to de-duplicate content.
    pub signature: Option<u64>,
    #[cfg(feature = "chrome")]
    /// All of the response events mapped with the amount of bytes used.
    pub response_map: Option<hashbrown::HashMap<String, f64>>,
    #[cfg(feature = "chrome")]
    /// All of the request events mapped with the time period of the event sent.
    pub request_map: Option<hashbrown::HashMap<String, f64>>,
    /// The anti-bot tech used.
    pub anti_bot_tech: AntiBotTech,
    /// Page metadata.
    pub metadata: Option<Box<Metadata>>,
    /// Whether the response content was truncated due to a stream error,
    /// chunk idle timeout, or Content-Length mismatch.
    pub content_truncated: bool,
    /// Whether a proxy was configured for this request.
    /// When true, 401 responses are retried (proxy rotation may fix auth).
    pub proxy_configured: bool,
    /// The profile key identifying which retry profile was used to fetch this page.
    /// Set by a [`RetryStrategy`](crate::retry_strategy::RetryStrategy); `None` when
    /// using the default simple retry counter.
    pub profile_key: Option<CompactString>,
    /// Whether the content is a binary file (image, PDF, etc.).
    /// Set once when HTML bytes are first available so the flag remains
    /// accurate after content is spooled to disk.
    pub binary_file: bool,
    /// Whether the HTML bytes are valid UTF-8.
    /// Set once when bytes are first available so subsequent accesses
    /// (including after disk spool) can skip re-validation.
    pub(crate) is_valid_utf8: bool,
    /// Whether the content starts with an XML declaration (`<?xml`).
    /// Set once when bytes are first available so the flag survives disk
    /// spooling and we can route to the XML parser without re-checking bytes.
    pub(crate) is_xml: bool,
    #[cfg(feature = "parallel_backends")]
    /// Identifies which backend produced this page (e.g. "primary",
    /// "cdp", "servo"). `None` when parallel backends are not active.
    pub backend_source: Option<crate::compact_str::CompactString>,
    #[cfg(feature = "balance")]
    /// Guard holding the path to a disk-spooled HTML file.  When the guard
    /// is dropped the temporary file is automatically deleted and the global
    /// byte counter stays consistent.  When set, `html` is `None` and
    /// content accessors transparently reload from this path.
    pub(crate) html_spool_path: Option<HtmlSpoolGuard>,
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    /// Whether this page's HTML bytes are currently tracked in the global
    /// `TOTAL_HTML_BYTES_IN_MEMORY` counter.  Set to `true` in `build()`
    /// when `track_bytes_add` is called; cleared to `false` in
    /// `channel_send_page` after bytes are subtracted.  The `Drop` impl
    /// uses this flag to subtract leaked bytes from pages that are dropped
    /// without going through `channel_send_page` (e.g. during retries).
    pub(crate) balance_bytes_tracked: bool,
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    /// Cached byte length of the HTML content, set at spool time so that
    /// `size()` can return the length without any I/O when HTML is on disk.
    pub(crate) content_byte_len: usize,
}

/// Subtract leaked bytes from the global counter when a `Page` is dropped
/// without going through `channel_send_page`.  This prevents the
/// `TOTAL_HTML_BYTES_IN_MEMORY` counter from growing monotonically during
/// Chrome retries (where `page = next_page` drops the old page).
///
/// Pages that pass through `channel_send_page` have `balance_bytes_tracked`
/// cleared to `false`, so their `Drop` is a no-op.
#[cfg(all(feature = "balance", not(feature = "decentralized")))]
impl Drop for Page {
    #[inline]
    fn drop(&mut self) {
        if self.balance_bytes_tracked {
            if let Some(ref html) = self.html {
                let len = html.len();
                if len > 0 {
                    crate::utils::html_spool::track_bytes_sub(len);
                }
            }
        }
    }
}

/// Represent a page visited.
#[cfg(feature = "decentralized")]
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// The bytes of the resource.
    pub(crate) html: Option<bytes::Bytes>,
    /// Base absolute url for page.
    pub(crate) base: Option<Url>,
    /// The raw url for the page. Useful since Url::parse adds a trailing slash.
    pub(crate) url: String,
    /// The headers of the page request response.
    pub headers: Option<reqwest::header::HeaderMap>,
    #[cfg(feature = "remote_addr")]
    /// The remote address of the page.
    pub remote_addr: Option<core::net::SocketAddr>,
    #[cfg(feature = "cookies")]
    /// The cookies of the page request response.
    pub cookies: Option<reqwest::header::HeaderMap>,
    /// The status code of the page request.
    pub status_code: StatusCode,
    /// The error of the request if any.
    pub error_status: Option<String>,
    /// The current links for the page.
    pub links: HashSet<CaseInsensitiveString>,
    /// The external urls to group with the domain.
    pub external_domains_caseless: Arc<HashSet<CaseInsensitiveString>>,
    /// The final destination of the page if redirects were performed [Unused].
    pub final_redirect_destination: Option<String>,
    #[cfg(feature = "time")]
    /// The duration from start of parsing to end of gathering links.
    duration: Option<Instant>,
    #[cfg(feature = "chrome")]
    /// The screenshot bytes of the page.
    pub screenshot_bytes: Option<Vec<u8>>,
    #[cfg(feature = "openai")]
    /// The credits used from OpenAI in order.
    pub openai_credits_used: Option<Vec<crate::features::openai_common::OpenAIUsage>>,
    #[cfg(feature = "openai")]
    /// The extra data from the AI, example extracting data etc...
    pub extra_ai_data: Option<Vec<AIResults>>,
    #[cfg(feature = "gemini")]
    /// The credits used from Gemini in order.
    pub gemini_credits_used: Option<Vec<crate::features::gemini_common::GeminiUsage>>,
    #[cfg(feature = "gemini")]
    /// The extra data from the Gemini AI.
    pub extra_gemini_data: Option<Vec<AIResults>>,
    /// The usage from remote multimodal automation (extraction, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub remote_multimodal_usage: Option<Vec<crate::features::automation::AutomationUsage>>,
    /// The extra data from the remote multimodal automation (extraction results, etc.).
    /// Works with both Chrome and HTTP-only crawls.
    pub extra_remote_multimodal_data: Option<Vec<AutomationResults>>,
    /// URLs requested by automation to spawn as additional pages.
    pub spawn_pages: Option<Vec<String>>,
    /// Additional content keyed by return format (e.g. `"markdown"`, `"text"`).
    /// Populated when multiple formats are requested via
    /// [`SpiderCloudConfig::with_return_formats`](crate::configuration::SpiderCloudConfig::with_return_formats).
    #[cfg(feature = "spider_cloud")]
    pub content_map: Option<hashbrown::HashMap<String, bytes::Bytes>>,
    /// The links found on the page. Unused until we can structure the buffers to match.
    pub page_links: Option<Box<HashSet<CaseInsensitiveString>>>,
    /// The request should retry.
    pub should_retry: bool,
    /// A WAF was found on the page.
    pub waf_check: bool,
    /// The total byte transferred for the page. Mainly used for chrome events.
    pub bytes_transferred: Option<f64>,
    /// The page was blocked from crawling usual from using website::on_should_crawl_callback.
    pub blocked_crawl: bool,
    /// The signature of the page to de-duplicate content.
    pub signature: Option<u64>,
    #[cfg(feature = "chrome")]
    /// All of the response events mapped with the amount of bytes used.
    pub response_map: Option<hashbrown::HashMap<String, f64>>,
    #[cfg(feature = "chrome")]
    /// All of the request events mapped with the time period of the event sent.
    pub request_map: Option<hashbrown::HashMap<String, f64>>,
    /// The anti-bot tech used.
    pub anti_bot_tech: AntiBotTech,
    /// Page metadata.
    pub metadata: Option<Box<Metadata>>,
    /// Whether the response content was truncated due to a stream error,
    /// chunk idle timeout, or Content-Length mismatch.
    pub content_truncated: bool,
    /// Whether a proxy was configured for this request.
    /// When true, 401 responses are retried (proxy rotation may fix auth).
    pub proxy_configured: bool,
    /// The profile key identifying which retry profile was used to fetch this page.
    /// Set by a [`RetryStrategy`](crate::retry_strategy::RetryStrategy); `None` when
    /// using the default simple retry counter.
    pub profile_key: Option<CompactString>,
    /// Whether the content is a binary file (image, PDF, etc.).
    /// Set once when HTML bytes are first available so the flag remains
    /// accurate after content is spooled to disk.
    pub binary_file: bool,
    /// Whether the HTML bytes are valid UTF-8.
    /// Set once when bytes are first available so subsequent accesses
    /// (including after disk spool) can skip re-validation.
    pub(crate) is_valid_utf8: bool,
    /// Whether the content starts with an XML declaration (`<?xml`).
    /// Set once when bytes are first available so the flag survives disk
    /// spooling and we can route to the XML parser without re-checking bytes.
    pub(crate) is_xml: bool,
    #[cfg(feature = "parallel_backends")]
    /// Identifies which backend produced this page (e.g. "primary",
    /// "cdp", "servo"). `None` when parallel backends are not active.
    pub backend_source: Option<crate::compact_str::CompactString>,
}

/// Assign properties from a new page.
#[cfg(feature = "smart")]
pub fn page_assign(page: &mut Page, mut new_page: Page) {
    if let Some(s) = new_page.final_redirect_destination.as_deref() {
        let bad = match s.as_bytes().first().copied() {
            None => true,
            Some(b'a') => s.starts_with("about:blank"),
            Some(b'c') => s.starts_with("chrome-error://chromewebdata"),
            _ => false,
        };
        if !bad {
            page.final_redirect_destination = Some(s.into());
        }
    }

    let chrome_default_empty_200 =
        new_page.status_code == 200 && new_page.bytes_transferred.is_none() && new_page.is_empty();

    page.anti_bot_tech = new_page.anti_bot_tech;
    page.base = std::mem::take(&mut new_page.base);
    page.blocked_crawl = new_page.blocked_crawl;

    if !chrome_default_empty_200 {
        page.status_code = new_page.status_code;
        page.bytes_transferred = new_page.bytes_transferred;
        if new_page.html.is_some() {
            // Transfer byte tracking ownership from new_page to page.
            #[cfg(all(feature = "balance", not(feature = "decentralized")))]
            {
                // Subtract old page bytes if tracked.
                if page.balance_bytes_tracked {
                    if let Some(ref old_html) = page.html {
                        crate::utils::html_spool::track_bytes_sub(old_html.len());
                    }
                }
                // Transfer the tracking flag from new_page.
                page.balance_bytes_tracked = new_page.balance_bytes_tracked;
                new_page.balance_bytes_tracked = false;
            }
            page.html = std::mem::take(&mut new_page.html);
            page.is_valid_utf8 = new_page.is_valid_utf8;
            page.is_xml = new_page.is_xml;
        }
    } else {
        // Chrome returned 200 with no content — mark for retry so the outer
        // loop re-fetches instead of silently accepting an empty page.
        page.should_retry = true;
    }

    #[cfg(feature = "remote_addr")]
    {
        page.remote_addr = new_page.remote_addr;
    }
    #[cfg(feature = "time")]
    {
        page.duration = new_page.duration;
    }
    #[cfg(feature = "page_error_status_details")]
    {
        page.error_status = std::mem::take(&mut new_page.error_status);
    }

    #[cfg(feature = "chrome")]
    {
        page.request_map = std::mem::take(&mut new_page.request_map);
        page.response_map = std::mem::take(&mut new_page.response_map);
    }

    #[cfg(feature = "cookies")]
    {
        if new_page.cookies.is_some() {
            page.cookies = std::mem::take(&mut new_page.cookies);
        }
    }
    if new_page.headers.is_some() {
        page.headers = std::mem::take(&mut new_page.headers);
    }

    page.waf_check = new_page.waf_check;
    page.should_retry = new_page.should_retry;
    page.signature = new_page.signature;
    if let Some(mut new_spawn_pages) = std::mem::take(&mut new_page.spawn_pages) {
        /// Max URLs a single page can accumulate via automation spawn_pages.
        const MAX_SPAWN_PAGES: usize = 1000;
        match page.spawn_pages.as_mut() {
            Some(existing) => {
                let remaining = MAX_SPAWN_PAGES.saturating_sub(existing.len());
                new_spawn_pages.truncate(remaining);
                existing.append(&mut new_spawn_pages);
            }
            None => {
                new_spawn_pages.truncate(MAX_SPAWN_PAGES);
                page.spawn_pages = Some(new_spawn_pages);
            }
        }
    }
    page.metadata = std::mem::take(&mut new_page.metadata);
}

/// Validate link and push into the map
pub(crate) fn validate_link<
    A: PartialEq + Eq + std::hash::Hash + From<String> + for<'a> From<&'a str>,
>(
    base: &Option<&Url>,
    href: &str,
    base_domain: &CompactString,
    parent_host: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
    links_pages: &mut Option<HashSet<A>>,
) -> Option<Url> {
    if let Some(b) = base {
        let abs = convert_abs_path(b, href);

        if let Some(link_map) = links_pages {
            link_map.insert(A::from(href));
        }

        let scheme = abs.scheme();

        if scheme == "https" || scheme == "http" {
            let host_name = abs.host_str();

            let mut can_process = parent_host_match(
                host_name,
                base_domain,
                parent_host,
                base_input_domain,
                sub_matcher,
            );

            // attempt to check if domain matches with port.
            if !can_process && host_name.is_some() && abs.port().is_some() {
                if let Some(host) = host_name {
                    let hname =
                        string_concat!(host, ":", abs.port().unwrap_or_default().to_string());
                    can_process = parent_host_match(
                        Some(&hname),
                        base_domain,
                        parent_host,
                        base_input_domain,
                        sub_matcher,
                    );
                }
            }

            if !can_process && host_name.is_some() && !external_domains_caseless.is_empty() {
                can_process = external_domains_caseless
                    .contains::<CaseInsensitiveString>(&host_name.unwrap_or_default().into())
                    || external_domains_caseless
                        .contains::<CaseInsensitiveString>(&CASELESS_WILD_CARD);
            }
            if can_process {
                return Some(abs);
            }
        }
    }
    None
}

/// determine a url is relative page
pub(crate) fn relative_directory_url(href: &str) -> bool {
    if href.starts_with("./") || href.starts_with("//") || href.starts_with("../") {
        true
    } else {
        let network_capable = networking_capable(href);

        if network_capable {
            false
        } else {
            !href.starts_with("/")
        }
    }
}

/// Validate link and push into the map without extended verify.
pub(crate) fn push_link<
    A: PartialEq + Eq + std::hash::Hash + From<String> + for<'a> From<&'a str>,
>(
    base: &Option<&Url>,
    href: &str,
    map: &mut HashSet<A>,
    base_domain: &CompactString,
    parent_host: &CompactString,
    parent_host_scheme: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
    links_pages: &mut Option<HashSet<A>>,
) {
    let abs = validate_link(
        base,
        href,
        base_domain,
        parent_host,
        base_input_domain,
        sub_matcher,
        external_domains_caseless,
        links_pages,
    );

    if let Some(mut abs) = abs {
        if abs.scheme() != parent_host_scheme.as_str() {
            let _ = abs.set_scheme(parent_host_scheme.as_str());
        }
        map.insert(A::from(abs.as_str()));
    }
}

/// Validate link and push into the map
pub(crate) fn push_link_verify<
    A: PartialEq + Eq + std::hash::Hash + From<String> + for<'a> From<&'a str>,
>(
    base: &Option<&Url>,
    href: &str,
    map: &mut HashSet<A>,
    base_domain: &CompactString,
    parent_host: &CompactString,
    parent_host_scheme: &CompactString,
    base_input_domain: &CompactString,
    sub_matcher: &CompactString,
    external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
    full_resources: bool,
    links_pages: &mut Option<HashSet<A>>,
    verify: bool,
) {
    let abs = validate_link(
        base,
        href,
        base_domain,
        parent_host,
        base_input_domain,
        sub_matcher,
        external_domains_caseless,
        links_pages,
    );
    if let Some(mut abs) = abs {
        if abs.scheme() != parent_host_scheme.as_str() {
            let _ = abs.set_scheme(parent_host_scheme.as_str());
        }
        if verify {
            push_link_check(&mut abs, map, full_resources, &mut true);
        } else {
            map.insert(A::from(abs.as_str()));
        }
    }
}

/// Determine if a url is an asset.
pub fn is_asset_url(url: &str) -> bool {
    if let Some(position) = url.rfind('.') {
        if url.len() - position >= 3 {
            return is_ignored_extension(&url[position + 1..]);
        }
    }
    false
}

/// Validate link and push into the map checking if asset
pub(crate) fn push_link_check<
    A: PartialEq + Eq + std::hash::Hash + From<String> + for<'a> From<&'a str>,
>(
    abs: &mut Url,
    map: &mut HashSet<A>,
    full_resources: bool,
    can_process: &mut bool,
) {
    let hchars = abs.path();

    // check if the file is a resource and block if it is
    if let Some(position) = hchars.rfind('.') {
        let hlen = hchars.len();
        let has_asset = hlen - position;

        if has_asset >= 3 {
            let next_position = position + 1;

            if !full_resources && is_ignored_extension(&hchars[next_position..]) {
                *can_process = false;
            }
        }
    }

    if *can_process {
        map.insert(A::from(abs.as_str()));
    }
}

/// get the clean domain name
pub(crate) fn domain_name(domain: &Url) -> &str {
    domain.host_str().unwrap_or_default()
}

/// Extract the root domain from a hostname in a single pass.
/// "sub.example.com" → "example.com", "example.com" → "example", "localhost" → "localhost"
#[inline]
fn extract_root_domain(domain: &str) -> &str {
    let bytes = domain.as_bytes();

    // SIMD reverse scan for dots via memchr.
    if let Some(last_dot) = memchr::memrchr(b'.', bytes) {
        if let Some(second_last_dot) = memchr::memrchr(b'.', &bytes[..last_dot]) {
            // "sub.example.com" → "example.com"
            &domain[second_last_dot + 1..]
        } else {
            // "example.com" → "example"
            &domain[..last_dot]
        }
    } else {
        // no dots — "localhost"
        domain
    }
}

/// Check for subdomain matches by comparing root domains.
#[inline]
#[cfg_attr(not(test), allow(dead_code))]
fn is_subdomain(subdomain: &str, domain: &str) -> bool {
    extract_root_domain(subdomain) == extract_root_domain(domain)
}

/// Validation to match a domain to parent host and the top level redirect
/// for the crawl. Short-circuits on exact match (most common case).
pub(crate) fn parent_host_match(
    host_name: Option<&str>,
    base_domain: &str,
    parent_host: &CompactString,
    base_host: &CompactString,
    sub_matcher: &CompactString,
) -> bool {
    match host_name {
        Some(host) => {
            // Fast path: exact match (covers ~80% of same-domain links).
            if parent_host.eq(&host) || base_host.eq(&host) {
                return true;
            }

            if base_domain.is_empty() {
                return false;
            }

            // Extract host's root domain once, compare against both targets.
            let host_root = extract_root_domain(host);
            extract_root_domain(parent_host) == host_root
                || extract_root_domain(sub_matcher) == host_root
        }
        _ => false,
    }
}

/// html selector for valid web pages for domain.
pub(crate) fn get_page_selectors_base(u: &str, subdomains: bool, tld: bool) -> RelativeSelectors {
    let dname = get_domain_from_url(u);
    let host_name = CompactString::from(dname);

    let scheme = if u.starts_with("https://") {
        "https"
    } else if u.starts_with("http://") {
        "http"
    } else if u.starts_with("file://") {
        "file"
    } else if u.starts_with("wss://") {
        "wss"
    } else if u.starts_with("ws://") {
        "ws"
    } else {
        // default to https
        "https"
    };

    if tld || subdomains {
        let dname = if tld {
            extract_root_domain(dname)
        } else {
            dname
        };

        (
            dname.into(), // match for tlds or subdomains
            smallvec::SmallVec::from([host_name, CompactString::from(scheme)]),
            CompactString::default(),
        )
    } else {
        (
            CompactString::default(),
            smallvec::SmallVec::from([host_name, CompactString::from(scheme)]),
            CompactString::default(),
        )
    }
}

/// html selector for valid web pages for domain.
pub fn get_page_selectors(url: &str, subdomains: bool, tld: bool) -> RelativeSelectors {
    get_page_selectors_base(url, subdomains, tld)
}

#[cfg(not(feature = "decentralized"))]
/// Is the resource valid?
pub fn validate_empty(content: &Option<Vec<u8>>, is_success: bool) -> bool {
    match &content {
        Some(content) => {
            // is_success && content.starts_with(br#"<html style=\"height:100%\"><head><META NAME=\"ROBOTS\" CONTENT=\"NOINDEX, NOFOLLOW\"><meta name=\"format-detection\" content=\"telephone=no\"><meta name=\"viewport\" content=\"initial-scale=1.0\"><meta http-equiv=\"X-UA-Compatible\" content=\"IE=edge,chrome=1\"></head><body style=\"margin:0px;height:100%\"><iframe id=\"main-iframe\" src=\"/_Incapsula_"#)
            !( content.is_empty() || content.starts_with(b"<html><head></head><body></body></html>") || is_success &&
                     content.starts_with(b"<html>\r\n<head>\r\n<META NAME=\"robots\" CONTENT=\"noindex,nofollow\">\r\n<script src=\"/") &&
                      content.ends_with(b"\">\r\n</script>\r\n<body>\r\n</body></html>\r\n")
                || is_chrome_error_page(content))
        }
        _ => false,
    }
}

/// Returns `true` if the HTML content is a Chrome browser error page
/// (e.g. ERR_TUNNEL_CONNECTION_FAILED, ERR_NAME_NOT_RESOLVED, etc.).
///
/// Chrome renders these pages locally when proxy/network errors occur.
/// They arrive with HTTP 200 and ~157KB of content (CSS, dino game JS,
/// base64 error images). Detection uses the structural tail: Chrome error
/// pages always end with `};</script></html>` (no `</body>` tag) and
/// contain `"errorCode":"ERR` in the `loadTimeDataRaw` JSON blob.
///
/// Two checks, both on the last 4KB only — O(1) for all non-error pages:
/// 1. `ends_with(b"};</script></html>")` — Chrome error page structure
/// 2. `"errorCode":"ERR` present — Chrome-internal JSON key
///
/// Zero false positives: real websites always have `</body>` before
/// `</html>`, and never contain `loadTimeDataRaw` with `errorCode`.
#[cfg(not(feature = "decentralized"))]
#[inline]
pub fn is_chrome_error_page(content: &[u8]) -> bool {
    const TAIL: &[u8] = b"};</script></html>";
    const NEEDLE: &[u8] = b"\"errorCode\":\"ERR";

    if content.len() < 500 {
        return false;
    }

    // Trim trailing whitespace for ends_with
    let mut end = content.len();
    while end > 0 && content[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let trimmed = &content[..end];

    if !trimmed.ends_with(TAIL) {
        return false;
    }

    // Confirm errorCode in the last 4KB
    let region = if trimmed.len() > 4096 {
        &trimmed[trimmed.len() - 4096..]
    } else {
        trimmed
    };
    memchr::memmem::find(region, NEEDLE).is_some()
}

/// Extract the `errorCode` value (e.g. `"ERR_NAME_NOT_RESOLVED"`) from a
/// Chrome error page's `loadTimeDataRaw` JSON blob. Scans only the final
/// 4KB to stay O(1) on large responses. Returns `None` when the needle is
/// absent or the value is not valid UTF-8.
#[inline]
pub fn extract_chrome_error_code(content: &[u8]) -> Option<&str> {
    const NEEDLE: &[u8] = b"\"errorCode\":\"";

    let region = if content.len() > 4096 {
        &content[content.len() - 4096..]
    } else {
        content
    };

    let start = memchr::memmem::find(region, NEEDLE)? + NEEDLE.len();
    let rest = region.get(start..)?;
    let end = memchr::memchr(b'"', rest)?;
    std::str::from_utf8(&rest[..end]).ok()
}

/// Return `true` when a Chrome `net::ERR_*` failure_text (or an `errorCode`
/// extracted from a rendered Chrome error page) represents a permanent
/// hostname-resolution failure — i.e. the DNS record does not exist and no
/// amount of proxy/browser rotation will change that.
///
/// Matches the two Chrome net errors that surface a missing hostname:
/// - `ERR_NAME_NOT_RESOLVED`   — classic NXDOMAIN / NOERROR-with-no-A case
/// - `ERR_NAME_RESOLUTION_FAILED` — resolver rejected the name outright
///
/// Transient DNS conditions (timeouts, malformed responses, resolver 5xx) are
/// intentionally excluded so they remain retryable.
#[inline]
pub fn is_chrome_name_resolution_error(code: &str) -> bool {
    // Accept both the `net::ERR_*` failure_text form and the bare
    // `ERR_*` errorCode form used in rendered error pages.
    let trimmed = code.strip_prefix("net::").unwrap_or(code);
    matches!(
        trimmed,
        "ERR_NAME_NOT_RESOLVED" | "ERR_NAME_RESOLUTION_FAILED"
    )
}

/// Return `true` when a Chrome `net::ERR_*` failure_text (or an `errorCode`
/// extracted from a rendered Chrome error page) represents a **target-side**
/// permanent failure — the destination itself is unreachable / refused /
/// doesn't exist / can't be reached securely, so retrying through a different
/// proxy or connection will not help.
///
/// Superset of [`is_chrome_name_resolution_error`]. Matches:
/// - `ERR_NAME_NOT_RESOLVED`      — classic NXDOMAIN
/// - `ERR_NAME_RESOLUTION_FAILED` — resolver rejected the name outright
/// - `ERR_ADDRESS_UNREACHABLE`    — no route to host; also emitted by Chrome
///   when a SOCKS proxy returns reply 0x04 ("host unreachable") for the
///   target, which is a target-side failure even though it arrives via the
///   proxy channel
/// - `ERR_CONNECTION_REFUSED`     — origin closed the port with RST / ICMP;
///   the host is up but no service is listening, which no proxy swap will fix
/// - `ERR_HTTP2_INADEQUATE_TRANSPORT_SECURITY` — h2 refused over weak TLS;
///   destination requires a TLS profile the client cannot meet, retry futile
/// - `ERR_INVALID_URL`, `ERR_UNSAFE_PORT`, `ERR_DISALLOWED_URL_SCHEME`,
///   `ERR_UNKNOWN_URL_SCHEME` — Chrome refuses to navigate to the URL
///   itself (malformed / blocked port / scheme not supported); retrying the
///   same URL will deterministically fail the same way
///
/// Transient conditions (timeouts, resets mid-stream, proxy failures, generic
/// cert issues) are intentionally excluded so they remain retryable against a
/// different proxy or connection.
#[inline]
pub fn is_chrome_permanent_failure(code: &str) -> bool {
    let trimmed = code.strip_prefix("net::").unwrap_or(code);
    matches!(
        trimmed,
        "ERR_NAME_NOT_RESOLVED"
            | "ERR_NAME_RESOLUTION_FAILED"
            | "ERR_ADDRESS_UNREACHABLE"
            | "ERR_CONNECTION_REFUSED"
            | "ERR_HTTP2_INADEQUATE_TRANSPORT_SECURITY"
            | "ERR_INVALID_URL"
            | "ERR_UNSAFE_PORT"
            | "ERR_DISALLOWED_URL_SCHEME"
            | "ERR_UNKNOWN_URL_SCHEME"
    )
}

/// Map a Chrome `net::ERR_*` permanent-failure code to the spider HTTP
/// status code that records WHY it was permanent. Callers should first gate
/// on [`is_chrome_permanent_failure`]; this helper assumes the code is
/// already known to be permanent and is only picking between three buckets:
/// - **525** (DNS)                — `ERR_NAME_*`
/// - **526** (origin unreachable) — `ERR_ADDRESS_UNREACHABLE`,
///   `ERR_CONNECTION_REFUSED`, `ERR_HTTP2_INADEQUATE_TRANSPORT_SECURITY`
///   (h2 over weak TLS — same "destination not reachable from this client"
///   semantics as the SSL handshake bucket)
/// - **400** (malformed request)  — `ERR_INVALID_URL`, `ERR_UNSAFE_PORT`,
///   `ERR_DISALLOWED_URL_SCHEME`, `ERR_UNKNOWN_URL_SCHEME` (the URL itself
///   is the problem, not the destination)
///
/// Unknown (non-permanent) codes fall back to 525 — `is_retryable_status`
/// excludes 525 / 526 / 400 so any of them still halts the retry loop.
/// Keeping 525 as the default matches the historical behavior when only
/// `is_chrome_name_resolution_error` existed — important for downstream
/// callers that pattern-match on 525 specifically.
#[inline]
pub(crate) fn chrome_permanent_failure_status(code: &str) -> StatusCode {
    let trimmed = code.strip_prefix("net::").unwrap_or(code);
    match trimmed {
        "ERR_ADDRESS_UNREACHABLE"
        | "ERR_CONNECTION_REFUSED"
        | "ERR_HTTP2_INADEQUATE_TRANSPORT_SECURITY" => *ADDRESS_UNREACHABLE_ERROR,
        "ERR_INVALID_URL"
        | "ERR_UNSAFE_PORT"
        | "ERR_DISALLOWED_URL_SCHEME"
        | "ERR_UNKNOWN_URL_SCHEME" => StatusCode::BAD_REQUEST,
        _ => *DNS_RESOLVE_ERROR,
    }
}

/// Extract a specific type of error from a chain of errors.
#[cfg(not(feature = "decentralized"))]
fn extract_specific_error<'a, T: std::error::Error + 'static>(
    error: &'a (dyn std::error::Error + 'static),
) -> Option<&'a T> {
    let mut current_error = Some(error);
    while let Some(err) = current_error {
        if let Some(desired_error) = err.downcast_ref::<T>() {
            return Some(desired_error);
        }
        current_error = err.source();
    }
    None
}

/// Determine if the response is goaway and should retry.
/// Covers transient HTTP/2 errors where a retry is likely to succeed:
/// - GO_AWAY + NO_ERROR: graceful server shutdown (load balancer rotation)
/// - REFUSED_STREAM: server rejected the stream before processing (RFC 7540
///   guarantees the request was not handled — safe to retry)
/// - ENHANCE_YOUR_CALM: server-side rate limiting (HTTP/2 equivalent of 429)
///
/// `INTERNAL_ERROR` is intentionally excluded: the server may have partially
/// processed the request, and the condition is often deterministic (server
/// bug rather than transient noise). Retrying just adds load on a broken
/// endpoint without a meaningful chance of success.
#[cfg(not(feature = "decentralized"))]
fn should_attempt_retry(error: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(e) = extract_specific_error::<h2::Error>(error) {
        if e.is_go_away() && e.is_remote() && e.reason() == Some(h2::Reason::NO_ERROR) {
            return true;
        }
        if e.is_remote() {
            if let Some(reason) = e.reason() {
                return matches!(
                    reason,
                    h2::Reason::REFUSED_STREAM | h2::Reason::ENHANCE_YOUR_CALM
                );
            }
        }
    }
    false
}

/// Map an HTTP/2 transport-level error to a permanent spider status code when
/// the underlying h2 reason is one the destination will keep emitting on
/// every retry. Returns `None` when no h2 error is in the chain or the reason
/// is transient.
///
/// Permanent reasons:
/// - `INADEQUATE_SECURITY` — server requires a stronger TLS profile than the
///   client offers; rotating proxies won't change the client's TLS stack.
///   Bucketed with the SSL handshake family at 526.
/// - `HTTP_1_1_REQUIRED`   — server explicitly refuses HTTP/2 for this
///   resource. Reqwest will not transparently downgrade an in-flight request,
///   so retrying via h2 keeps failing. Mapped to 526 too — same "this client
///   cannot reach this destination" semantics.
///
/// Walks the source chain via `extract_specific_error` (no allocation, no
/// locks). Hot path runs only on the error branch; never invoked on the
/// happy path of a successful request.
#[cfg(not(feature = "decentralized"))]
#[inline]
fn h2_permanent_reason_status(err: &crate::client::Error) -> Option<StatusCode> {
    let h2_err = extract_specific_error::<h2::Error>(err)?;
    if !h2_err.is_remote() {
        return None;
    }
    let reason = h2_err.reason()?;
    if matches!(
        reason,
        h2::Reason::INADEQUATE_SECURITY | h2::Reason::HTTP_1_1_REQUIRED
    ) {
        Some(*ADDRESS_UNREACHABLE_ERROR)
    } else {
        None
    }
}

/// Get the error status of the page base.
#[cfg(not(feature = "decentralized"))]
fn get_error_status_base(
    should_retry: &mut bool,
    error_for_status: Option<Result<crate::utils::RequestResponse, RequestError>>,
) -> Option<RequestError> {
    match error_for_status {
        Some(e) => match e {
            Ok(_) => None,
            Err(er) => {
                // Compute the mapped status once and reuse it across every
                // branch below. Previously this was computed up to twice
                // per error (once through `is_dns_error` in the connect
                // fast-path, once in the catch-all fallback) — folding
                // them into one call keeps the error path cheap when the
                // full chain walk + io-error downcast get run.
                //
                // `is_retryable_status` (a few u16 compares) supersedes
                // the earlier `!is_dns_error` gate: it covers DNS (525),
                // address-unreachable (526), and redirect-cap (310) in a
                // single check, so adding new permanent codes to
                // `is_retryable_status` automatically propagates here.
                let mapped_status = get_error_http_status_code(&er);
                if er.is_timeout() || (er.is_connect() && is_retryable_status(mapped_status)) {
                    *should_retry = true;
                }
                if !*should_retry && should_attempt_retry(&er) {
                    *should_retry = true;
                }
                if let Some(status_code) = er.status() {
                    let retry = matches!(
                        status_code,
                        StatusCode::TOO_MANY_REQUESTS
                            | StatusCode::INTERNAL_SERVER_ERROR
                            | StatusCode::BAD_GATEWAY
                            | StatusCode::SERVICE_UNAVAILABLE
                            | StatusCode::GATEWAY_TIMEOUT
                    );
                    if retry {
                        *should_retry = true;
                    }
                }
                // Catch-all: errors that neither set `is_timeout` /
                // `is_connect` / `er.status()` nor matched an h2 reason
                // can still map to a retryable status (e.g. 598/599).
                if !*should_retry && is_retryable_status(mapped_status) {
                    *should_retry = true;
                }
                Some(er)
            }
        },
        _ => None,
    }
}

#[cfg(all(
    not(feature = "page_error_status_details"),
    not(feature = "decentralized")
))]
/// Get the error status of the page.
fn get_error_status(
    should_retry: &mut bool,
    error_for_status: Option<Result<crate::utils::RequestResponse, RequestError>>,
) -> Option<String> {
    get_error_status_base(should_retry, error_for_status).map(|e| e.to_string())
}

#[cfg(all(feature = "page_error_status_details", not(feature = "decentralized")))]
/// Get the error status of the page.
fn get_error_status(
    should_retry: &mut bool,
    error_for_status: Option<Result<crate::utils::RequestResponse, RequestError>>,
) -> Option<std::sync::Arc<reqwest::Error>> {
    get_error_status_base(should_retry, error_for_status).map(std::sync::Arc::new)
}

#[cfg(not(feature = "decentralized"))]
/// Instantiate a new page without scraping it and with the base URL parsed (used for testing purposes).
pub fn build_with_parse(url: &str, res: PageResponse) -> Page {
    let mut page = build(url, res);
    page.set_url_parsed_direct_empty();
    page
}

/// Instantiate a new page without scraping it and with the base URL parsed (used for testing purposes).
#[cfg(feature = "decentralized")]
pub fn build_with_parse(url: &str, res: PageResponse) -> Page {
    build(url, res)
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(not(feature = "decentralized"))]
pub fn build(url: &str, mut res: PageResponse) -> Page {
    use crate::utils::validation::is_false_403;

    // Chrome error pages (ERR_CONNECTION_RESET, ERR_TUNNEL_CONNECTION_FAILED, etc.)
    // return HTTP 200 with ~157KB of error page content. Reclassify to 599
    // (spider internal error) so all retry paths treat it as a failed crawl.
    // Content is preserved for debugging but status signals failure.
    //
    // When the rendered errorCode reports a permanent target-side failure
    // (DNS absent / address unreachable / connection refused), downgrade to
    // the appropriate non-retryable code — 525 for DNS or 526 for reachable-
    // but-refused — so the retry path treats it as permanent instead of the
    // generic 599 catch-all.
    let chrome_error =
        res.status_code.is_success() && res.content.as_deref().is_some_and(is_chrome_error_page);
    if chrome_error {
        let permanent_code = res
            .content
            .as_deref()
            .and_then(extract_chrome_error_code)
            .filter(|code| is_chrome_permanent_failure(code));
        res.status_code = if let Some(code) = permanent_code {
            chrome_permanent_failure_status(code)
        } else {
            StatusCode::from_u16(599).unwrap_or(StatusCode::BAD_GATEWAY)
        };
    }

    // Empty / shell body but success status — upstream returned 200 with
    // no usable payload (proxy edge-blocked, backend hiccup, blank shell).
    // Reclassify to 504 GATEWAY_TIMEOUT so downstream consumers see
    // failure on `status_code` instead of having to gate on
    // `should_retry` + `is_empty()` separately. Mirrors the chrome_error
    // pattern above; truncated bodies are preserved (a partial body is
    // real data, not a silent failure).
    //
    // **Metadata-first short-circuit**: under `balance` the spool writer
    // (`spool_html_to_disk`, since v2.51.66) refuses empty / shell HTML,
    // so a present `content_spool` is proof that real bytes exist on
    // disk. Treat that as `resource_found = true` *without* calling
    // `validate_empty` — never re-scan or re-read what the writer
    // already counted. As a side-benefit this fixes a latent issue
    // where spooled pages inherited `should_retry = true` via
    // `should_retry_empty_success` because `validate_empty` only saw
    // `res.content = None`.
    let success_initial = res.status_code.is_success() || res.status_code == StatusCode::NOT_FOUND;
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    let resource_found_initial = if res.content_spool.is_some() {
        true
    } else {
        validate_empty(&res.content, success_initial)
    };
    #[cfg(not(all(feature = "balance", not(feature = "decentralized"))))]
    let resource_found_initial = validate_empty(&res.content, success_initial);

    if !chrome_error
        && res.status_code.is_success()
        && !res.content_truncated
        && !resource_found_initial
    {
        res.status_code = StatusCode::GATEWAY_TIMEOUT;
    }

    let success = res.status_code.is_success() || res.status_code == StatusCode::NOT_FOUND;
    let resource_found = resource_found_initial;

    let status = res.status_code;

    // DNS resolve error (525) and address-unreachable / SSL-handshake (526)
    // are permanent — never retry. 501 / 505 / 511 are 5xx codes the server
    // emits to declare a deterministic "won't do" condition (method not
    // implemented / HTTP version mismatch / captive portal): retrying without
    // changing the request will keep failing.
    let should_retry_status = status != *DNS_RESOLVE_ERROR
        && status != *ADDRESS_UNREACHABLE_ERROR
        && status != StatusCode::NOT_IMPLEMENTED
        && status != StatusCode::HTTP_VERSION_NOT_SUPPORTED
        && status != StatusCode::NETWORK_AUTHENTICATION_REQUIRED
        && (status.is_server_error()
            || matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN | StatusCode::REQUEST_TIMEOUT
            ));

    let should_retry_resource = resource_found && !success && status != StatusCode::UNAUTHORIZED;

    // Success status but no usable content — the crawl likely failed silently
    // (e.g. upstream returned 200 with empty body or blank HTML shell).
    let should_retry_empty_success = success && !resource_found && !res.content_truncated;

    let should_retry_antibot_false_403 = res.anti_bot_tech != AntiBotTech::None
        && res.status_code.is_success()
        && is_false_403(
            res.content.as_deref(),
            res.headers
                .as_ref()
                .and_then(|h| h.get(reqwest::header::CONTENT_LANGUAGE))
                .and_then(|v| v.to_str().ok()),
        );

    let mut should_retry = should_retry_resource
        || should_retry_status
        || should_retry_empty_success
        || should_retry_antibot_false_403;

    let mut empty_page = false;

    if let Some(final_url) = &res.final_url {
        if final_url.starts_with("chrome-error://chromewebdata")
            || final_url.starts_with("about:blank")
        {
            should_retry = false;
            empty_page = true;
        }
    }

    // Cancel retry for legitimate 403s (Apache/nginx access denied) but keep
    // retrying when antibot tech is detected — proxy/browser rotation can bypass
    // WAF blocks (Cloudflare Bot Management, DataDome, Imperva, etc.).
    if should_retry
        && !resource_found
        && res.status_code == StatusCode::FORBIDDEN
        && res.headers.is_some()
        && res.anti_bot_tech == AntiBotTech::None
    {
        should_retry = false;
    }

    // ── Pre-spooled content path (balance + chrome + pressure) ────────
    //
    // When the fetch layer handed us a `SpooledContent` handle the HTML
    // is already on disk and every vital was computed inline with the
    // write.  Materialising the bytes again — even transiently — would
    // defeat the feature.  Fold the cached values straight onto the
    // page, skip the `html` buffer, and install the spool guard so
    // `HtmlSpoolGuard::Drop` still cleans up on the final drop.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    if let Some(spool) = res.content_spool.take() {
        // Pre-spooled pages already have a `hash_html`-equivalent
        // signature computed against their own normalised bytes — carry
        // it straight onto the page so the downstream signature sites
        // (see `website.rs`) see the same value they would have
        // computed from the in-memory bytes.  When the fetch layer
        // didn't pre-compute (legacy PageResponse.signature from earlier
        // code paths), fall back to that value.
        let precomputed_signature = spool.signature.or(res.signature);
        return Page {
            html: None,
            binary_file: spool.vitals.binary_file,
            is_valid_utf8: spool.vitals.is_valid_utf8,
            is_xml: spool.vitals.is_xml,
            headers: res.headers,
            #[cfg(feature = "remote_addr")]
            remote_addr: res.remote_addr,
            #[cfg(feature = "cookies")]
            cookies: res.cookies,
            url: url.into(),
            #[cfg(feature = "time")]
            duration: res.duration,
            status_code: res.status_code,
            error_status: get_error_status(&mut should_retry, res.error_for_status),
            final_redirect_destination: if empty_page { None } else { res.final_url },
            #[cfg(feature = "chrome")]
            chrome_page: None,
            #[cfg(feature = "chrome")]
            screenshot_bytes: res.screenshot_bytes,
            #[cfg(feature = "openai")]
            openai_credits_used: res.openai_credits_used,
            #[cfg(feature = "openai")]
            extra_ai_data: res.extra_ai_data,
            #[cfg(feature = "gemini")]
            gemini_credits_used: res.gemini_credits_used,
            #[cfg(feature = "gemini")]
            extra_gemini_data: res.extra_gemini_data,
            remote_multimodal_usage: res.remote_multimodal_usage,
            extra_remote_multimodal_data: res.extra_remote_multimodal_data,
            spawn_pages: res.spawn_pages,
            #[cfg(feature = "spider_cloud")]
            content_map: res.content_map,
            should_retry,
            waf_check: res.waf_check,
            bytes_transferred: res.bytes_transferred,
            blocked_crawl: false,
            signature: precomputed_signature,
            #[cfg(feature = "chrome")]
            response_map: res.response_map,
            #[cfg(feature = "chrome")]
            request_map: res.request_map,
            anti_bot_tech: res.anti_bot_tech,
            metadata: res.metadata,
            content_truncated: res.content_truncated,
            balance_bytes_tracked: false,
            base: None,
            external_domains_caseless: Default::default(),
            page_links: None,
            proxy_configured: false,
            profile_key: None,
            // Hook the guard to the current per-website spool dir (if
            // we're inside a website crawl scope) so the dir survives as
            // long as any page produced by the crawl is still live.
            // Outside a scope this resolves to `None` — identical to the
            // legacy global-dir behaviour, zero per-page cost.
            html_spool_path: Some(HtmlSpoolGuard::new(
                spool.path,
                crate::utils::html_spool::current_website_spool_dir(),
            )),
            content_byte_len: spool.vitals.byte_len,
            #[cfg(feature = "parallel_backends")]
            backend_source: None,
        };
    }

    let binary_file = res
        .content
        .as_deref()
        .is_some_and(crate::utils::is_binary_body);

    // Hot path: the HTTP and Chrome content producers populate
    // `res.is_valid_utf8` while the bytes are still warm in cache, so we
    // can avoid a second cold full-buffer scan here.  When a producer
    // didn't commit to a value (legacy paths, tests, small static
    // fillers), fall through to a one-shot validation — those payloads
    // are tiny and the cost is negligible.
    let is_valid_utf8 = match res.is_valid_utf8 {
        Some(v) => v,
        None => res
            .content
            .as_deref()
            .is_some_and(|b| simdutf8::basic::from_utf8(b).is_ok()),
    };

    let is_xml = res
        .content
        .as_deref()
        .is_some_and(|b| b.starts_with(b"<?xml"));

    // Cache the byte length at build time so `size()`, budget checks, and
    // accounting stay correct for both in-memory and later-spooled pages
    // without ever needing to peek at the content again.
    let content_byte_len = res.content.as_ref().map_or(0, |c| c.len());

    // Track in-memory HTML bytes for the balance budget check.
    // Subtracted when the page is spooled to disk, broadcast via
    // channel_send_page, or dropped (via the Page Drop impl).
    #[cfg(feature = "balance")]
    let balance_has_bytes = if content_byte_len > 0 {
        crate::utils::html_spool::track_bytes_add(content_byte_len);
        true
    } else {
        false
    };

    Page {
        html: res.content.map(bytes::Bytes::from),
        binary_file,
        is_valid_utf8,
        is_xml,
        headers: res.headers,
        #[cfg(feature = "remote_addr")]
        remote_addr: res.remote_addr,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        url: url.into(),
        #[cfg(feature = "time")]
        duration: res.duration,
        status_code: res.status_code,
        error_status: get_error_status(&mut should_retry, res.error_for_status),
        final_redirect_destination: if empty_page { None } else { res.final_url },
        #[cfg(feature = "chrome")]
        chrome_page: None,
        #[cfg(feature = "chrome")]
        screenshot_bytes: res.screenshot_bytes,
        #[cfg(feature = "openai")]
        openai_credits_used: res.openai_credits_used,
        #[cfg(feature = "openai")]
        extra_ai_data: res.extra_ai_data,
        #[cfg(feature = "gemini")]
        gemini_credits_used: res.gemini_credits_used,
        #[cfg(feature = "gemini")]
        extra_gemini_data: res.extra_gemini_data,
        remote_multimodal_usage: res.remote_multimodal_usage,
        extra_remote_multimodal_data: res.extra_remote_multimodal_data,
        spawn_pages: res.spawn_pages,
        #[cfg(feature = "spider_cloud")]
        content_map: res.content_map,
        should_retry,
        waf_check: res.waf_check,
        bytes_transferred: res.bytes_transferred,
        blocked_crawl: false,
        signature: res.signature,
        #[cfg(feature = "chrome")]
        response_map: res.response_map,
        #[cfg(feature = "chrome")]
        request_map: res.request_map,
        anti_bot_tech: res.anti_bot_tech,
        metadata: res.metadata,
        content_truncated: res.content_truncated,
        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        balance_bytes_tracked: balance_has_bytes,
        base: None,
        external_domains_caseless: Default::default(),
        page_links: None,
        proxy_configured: false,
        profile_key: None,
        #[cfg(feature = "balance")]
        html_spool_path: None,
        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        content_byte_len,
        #[cfg(feature = "parallel_backends")]
        backend_source: None,
    }
}

/// Instantiate a new page without scraping it (used for testing purposes).
#[cfg(feature = "decentralized")]
pub fn build(_: &str, res: PageResponse) -> Page {
    Page {
        html: res.content.map(bytes::Bytes::from),
        headers: res.headers,
        #[cfg(feature = "remote_addr")]
        remote_addr: res.remote_addr,
        #[cfg(feature = "cookies")]
        cookies: res.cookies,
        final_redirect_destination: res.final_url,
        status_code: res.status_code,
        metadata: res.metadata,
        spawn_pages: res.spawn_pages,
        content_truncated: res.content_truncated,
        error_status: match res.error_for_status {
            Some(e) => match e {
                Ok(_) => None,
                Err(er) => Some(er.to_string()),
            },
            _ => None,
        },
        ..Default::default()
    }
}

#[cfg(all(feature = "headers", feature = "cookies"))]
/// Re build the cookies.
pub fn build_cookie_header_from_set_cookie(page: &Page) -> Option<reqwest::header::HeaderValue> {
    use reqwest::header::HeaderValue;
    let mut cookie_pairs = Vec::with_capacity(8);

    if let Some(headers) = &page.headers {
        for cookie in headers.get_all(crate::client::header::SET_COOKIE).iter() {
            if let Ok(cookie_str) = cookie.to_str() {
                if let Ok(parsed) = cookie::Cookie::parse(cookie_str) {
                    cookie_pairs.push(format!("{}={}", parsed.name(), parsed.value()));
                }
            }
        }
    }

    if cookie_pairs.is_empty() {
        None
    } else {
        let cookie_header_str = cookie_pairs.join("; ");
        HeaderValue::from_str(&cookie_header_str).ok()
    }
}

#[cfg(not(all(feature = "headers", feature = "cookies")))]
/// Re build the cookies.
pub fn build_cookie_header_from_set_cookie(_page: &Page) -> Option<reqwest::header::HeaderValue> {
    None
}

/// Settings for streaming rewriter
#[derive(Debug, Default, Clone, Copy)]
pub struct PageLinkBuildSettings {
    /// If the SSG build is in progress.
    pub ssg_build: bool,
    /// If full resources should be included.
    pub full_resources: bool,
    /// TLD handling resources.
    pub tld: bool,
    /// Subdomain handling resources.
    pub subdomains: bool,
    /// De-duplication signature.
    pub normalize: bool,
    /// Skip link extraction (single-page crawls that don't need links).
    pub skip_links: bool,
}

impl PageLinkBuildSettings {
    /// New build link settings.
    pub(crate) fn new(ssg_build: bool, full_resources: bool) -> Self {
        Self {
            ssg_build,
            full_resources,
            ..Default::default()
        }
    }

    /// New build full link settings.
    pub(crate) fn new_full(
        ssg_build: bool,
        full_resources: bool,
        subdomains: bool,
        tld: bool,
        normalize: bool,
    ) -> Self {
        Self {
            ssg_build,
            full_resources,
            subdomains,
            tld,
            normalize,
            skip_links: false,
        }
    }
}

/// Get the content type from the responses
pub(crate) fn get_charset_from_content_type(
    headers: &reqwest::header::HeaderMap,
) -> Option<AsciiCompatibleEncoding> {
    use auto_encoder::encoding_rs;

    if let Some(content_type) = headers.get(reqwest::header::CONTENT_TYPE) {
        if let Ok(content_type_str) = content_type.to_str() {
            for part in content_type_str.split(';') {
                let part = part.trim();
                if part.len() >= 8 && part.as_bytes()[..8].eq_ignore_ascii_case(b"charset=") {
                    let stripped = &part[8..];
                    if let Some(encoding) = encoding_rs::Encoding::for_label(stripped.as_bytes()) {
                        if let Some(ascii_encoding) = AsciiCompatibleEncoding::new(encoding) {
                            return Some(ascii_encoding);
                        }
                    }
                }
            }
        }
    }

    None
}

#[cfg(feature = "chrome")]
/// Move the automation results from an existing metadata record into a freshly
/// built one. The source Option is about to be overwritten by the caller, so
/// taking ownership avoids cloning a potentially-large Vec<AutomationResults>
/// (each entry may hold base64 screenshot output).
pub(crate) fn set_metadata(mdata: &mut Option<Box<Metadata>>, metadata: &mut Metadata) {
    if let Some(mdata) = mdata {
        if mdata.automation.is_some() {
            metadata.automation = mdata.automation.take();
        }
    }
}

/// Set the metadata found on the page.

#[cfg(not(feature = "chrome"))]
pub(crate) fn set_metadata(_mdata: &mut Option<Box<Metadata>>, _metadata: &mut Metadata) {}

/// Check if urls are the same without the trailing slashes.
fn exact_url_match(url: &str, target_url: &str) -> bool {
    let end_target_slash = target_url.ends_with('/');
    let main_slash = url.ends_with('/');

    if end_target_slash && !main_slash {
        strip_trailing_slash(target_url) == url
    } else if !end_target_slash && main_slash {
        url == strip_trailing_slash(target_url)
    } else {
        url == target_url
    }
}

/// Strip end matching
fn strip_trailing_slash(s: &str) -> &str {
    if s.ends_with('/') {
        s.trim_end_matches('/')
    } else {
        s
    }
}

/// metadata handlers
pub(crate) fn metadata_handlers<'h>(
    meta_title: &'h mut Option<CompactString>,
    meta_description: &'h mut Option<CompactString>,
    meta_og_image: &'h mut Option<CompactString>,
) -> Vec<(
    std::borrow::Cow<'static, lol_html::Selector>,
    lol_html::send::ElementContentHandlers<'h>,
)> {
    vec![
        lol_html::text!("head title", |el| {
            let t = el.as_str();
            if !t.is_empty() {
                *meta_title = Some(t.into());
            }

            Ok(())
        }),
        lol_html::element!(r#"meta[name="description"]"#, |el| {
            if let Some(content) = el.get_attribute("content") {
                if !content.is_empty() {
                    *meta_description = Some(content.into());
                }
            }
            Ok(())
        }),
        lol_html::element!(r#"meta[property="og:image"]"#, |el| {
            if let Some(content) = el.get_attribute("content") {
                if !content.is_empty() {
                    *meta_og_image = Some(content.into());
                }
            }
            Ok(())
        }),
    ]
}

/// Type-erased streaming link/metadata extractor used by the Chrome
/// fetch chain.  Wraps a [`lol_html::send::HtmlRewriter`] behind a
/// `Box<dyn FnMut>` output sink so the struct itself is non-generic —
/// callers in `fetch_page_html_chrome_base` and friends can plumb
/// `Option<&mut ChromeStreamingExtractor<'_>>` without infecting the
/// signature with `OS`-generic parameters.
///
/// **Lifetime story.** `'h` ties together every borrow the caller's
/// lol_html closures hold (`&mut links`, `&mut links_pages`,
/// `&base_input_url`, `&mut meta_*`, …).  The macro that builds the
/// extractor owns those slots; the rewriter (and therefore the
/// extractor) cannot outlive them.  Passing
/// `Option<&'a mut ChromeStreamingExtractor<'h>>` through the async
/// fetch chain is sound as long as the macro awaits the chain inside
/// the same scope where the slots live — which is exactly how
/// `chrome_page_fetch!` is structured.
///
/// **Cancellation safety.** If the wrapping future is dropped
/// mid-stream, `Drop` runs on the rewriter (which releases its parser
/// state) and on every captured borrow.  No leak.
/// Zero-sized no-op output sink for [`ChromeStreamingExtractor`].
/// We never consume the rewriter's output bytes — chrome callers feed
/// the raw chromey chunks directly to their byte accumulator
/// (`collected: Vec<u8>` in [`crate::utils::fetch_chrome_html_streaming_into_writer`]).
/// Keeping the sink concrete (instead of `Box<dyn FnMut>`) eliminates
/// the one-per-page heap allocation the trait object would otherwise
/// require.
#[cfg(feature = "chrome")]
pub(crate) struct NoopOutputSink;

#[cfg(feature = "chrome")]
impl lol_html::OutputSink for NoopOutputSink {
    #[inline]
    fn handle_chunk(&mut self, _: &[u8]) {}
}

/// Magic-byte sniff window — first chunk's leading bytes feed
/// `auto_encoder::is_binary_file` to short-circuit the rewriter when
/// chrome returns binary asset content (image, PDF, font, archive).
/// 64 bytes is enough to cover every magic header in practice (PNG
/// = 8, PDF = 4, JPEG = 3, GIF = 6, ZIP = 4, etc.) without flushing
/// the L1 line.
#[cfg(feature = "chrome")]
const ASSET_SNIFF_BYTES: usize = 64;

#[cfg(feature = "chrome")]
#[allow(dead_code)] // wired in upcoming macro changes
pub(crate) struct ChromeStreamingExtractor<'h> {
    rewriter: lol_html::send::HtmlRewriter<'h, NoopOutputSink>,
    /// Tripped when a `write` returns Err (parser rejection / OOM)
    /// **or** when the first-chunk sniff detects binary asset bytes
    /// (PNG, JPEG, PDF, etc.).  In the asset case the rewriter is
    /// skipped from then on — chrome still streams the body to the
    /// caller's accumulator, but we don't waste CPU pushing binary
    /// bytes through `lol_html`.
    write_failed: bool,
    /// `false` until the first chunk arrives.  We sniff up to
    /// [`ASSET_SNIFF_BYTES`] of that chunk for binary magic bytes and
    /// set `write_failed` accordingly, then refuse the rewriter.
    sniffed: bool,
    /// Set by [`mark_streamed`] when the chrome chunk pump confirms it
    /// fed every chunk through to completion.  Without this flag,
    /// `end()` defaults to `false` so callers fall back to the legacy
    /// `page.links()` second-pass walk — the safe choice when the
    /// extractor was constructed but no streaming ran (e.g. XML
    /// carve-out, CDP-error fallback, post-multimodal refresh, fill-
    /// fetch, or the chrome events-failed branch).
    streamed_through: bool,
}

#[cfg(feature = "chrome")]
#[allow(dead_code)]
impl<'h> ChromeStreamingExtractor<'h> {
    /// Build the extractor from a handler vector produced by
    /// [`build_link_extract_handlers`].  The output sink is a no-op
    /// `Box<dyn FnMut>` — chrome callers consume the raw chromey
    /// chunks separately for downstream byte consumers.
    pub(crate) fn new(
        handlers: Vec<(
            std::borrow::Cow<'static, lol_html::Selector>,
            lol_html::send::ElementContentHandlers<'h>,
        )>,
        encoding: Option<lol_html::AsciiCompatibleEncoding>,
        adjust_charset_on_meta_tag: bool,
    ) -> Self {
        let settings = lol_html::send::Settings {
            element_content_handlers: handlers,
            adjust_charset_on_meta_tag,
            encoding: encoding.unwrap_or_else(lol_html::AsciiCompatibleEncoding::utf_8),
            ..lol_html::send::Settings::new_for_handler_types()
        };
        Self {
            rewriter: lol_html::send::HtmlRewriter::new(settings, NoopOutputSink),
            write_failed: false,
            sniffed: false,
            streamed_through: false,
        }
    }

    /// Feed a chunk to the rewriter.  Idempotent on prior failure —
    /// further writes are silently ignored after the first error so
    /// the chunk-pump stays cheap.  On the first chunk we sniff up to
    /// [`ASSET_SNIFF_BYTES`] of leading bytes via
    /// `auto_encoder::is_binary_file`; binary content (images, PDFs,
    /// fonts, archives) flips `write_failed` so `lol_html` is bypassed
    /// — the rewriter would just produce noise on non-text bytes.
    #[inline]
    pub(crate) fn write(&mut self, chunk: &[u8]) {
        if self.write_failed {
            return;
        }
        if !self.sniffed {
            self.sniffed = true;
            if !chunk.is_empty() {
                // Trim leading ASCII whitespace so a server-padded HTML
                // body (`\n\n\n\n<!doctype...`) isn't sniffed against the
                // wrong window.  When the chunk is whitespace-only, leave
                // the sniff for the next chunk.
                let trimmed = crate::utils::skip_leading_ascii_whitespace(chunk);
                if !trimmed.is_empty() {
                    let head_len = trimmed.len().min(ASSET_SNIFF_BYTES);
                    if auto_encoder::is_binary_file(&trimmed[..head_len]) {
                        // Asset detected mid-stream — invalidate the
                        // rewriter. Caller's `Vec<u8>` accumulator still
                        // captures the bytes for downstream consumers.
                        self.write_failed = true;
                        return;
                    }
                } else {
                    // Whitespace-only chunk: re-sniff on the next call.
                    self.sniffed = false;
                }
            }
        }
        if self.rewriter.write(chunk).is_err() {
            self.write_failed = true;
        }
    }

    /// Caller signal that the chrome chunk pump fed the rewriter every
    /// byte from the response stream.  Required before `end()` will
    /// report success — without it, the post-process layer falls back
    /// to the legacy second-pass so empty link sets aren't published.
    #[inline]
    pub(crate) fn mark_streamed(&mut self) {
        self.streamed_through = true;
    }

    /// Mark the extractor as stale because the page body was replaced
    /// after streaming completed (multimodal refresh, parallel-backend
    /// winner, etc.).  Forces `end()` to return `false` so the post-
    /// process layer re-extracts links from the new body.
    #[inline]
    pub(crate) fn invalidate(&mut self) {
        self.write_failed = true;
    }

    /// Finalize the rewriter.  Returns `true` only when every chunk
    /// fed cleanly, [`mark_streamed`] was called, **and** `end()`
    /// succeeded.  On `false`, the caller must fall back to a legacy
    /// `page.links()` second pass; the link sets are not safe to
    /// publish.
    pub(crate) fn end(self) -> bool {
        if self.write_failed || !self.streamed_through {
            return false;
        }
        self.rewriter.end().is_ok()
    }

    /// Whether the extractor is still in a clean state (no rewriter
    /// errors so far).  Useful for early-exit checks.
    #[inline]
    pub(crate) fn ok(&self) -> bool {
        !self.write_failed
    }
}

/// Borrow-only context bag for [`build_link_extract_handlers`].  Every
/// reference is owned by the caller for the lifetime of the rewriter;
/// this struct just bundles them so the helper signature stays sane.
///
/// Behaviour is byte-identical to the hand-rolled handler vectors at:
///   - `Page::new_page_streaming`            (page.rs ~2654)
///   - `Page::new_page_streaming_from_bytes` (page.rs ~3014)
///   - `Page::links_stream_base`             (page.rs ~4636)
///   - `Page::links_stream_base_from_disk`   (page.rs ~4805)
///   - `Page::links_stream_base_ssg`         (page.rs ~5168)
///   - `Page::links_stream_full_resource`    (page.rs ~6593)
///
/// SSG capture has two modes preserved verbatim from existing call
/// sites — `ssg_raw_src_cell` stores the unresolved `src` (caller
/// resolves later) and `ssg_resolved_path_cell` stores the
/// `convert_abs_path`-resolved value.  At most one of the two should
/// be `Some` at a time.
pub(crate) struct LinkExtractCtx<'h, A> {
    /// Domain selectors for `push_link` resolution.
    pub selectors: &'h RelativeSelectors,
    /// External-domain allowlist used by `push_link` for cross-host
    /// inclusion decisions.
    pub external_domains_caseless: &'h Arc<HashSet<CaseInsensitiveString>>,
    /// Output set populated by every matched anchor.
    pub map: &'h mut hashbrown::HashSet<A>,
    /// Optional secondary set for `return_page_links` mode — `None`
    /// when page-link tracking is disabled.
    pub links_pages: &'h mut Option<hashbrown::HashSet<A>>,
    /// `<base href>` capture cell.  Interior-mut so the base-element
    /// handler `set`s while the link handler reads.
    pub base_input_url: &'h tokio::sync::OnceCell<Url>,
    /// Caller-derived base URL for relative-link resolution
    /// (post-redirect domain or `prior_domain`).
    pub base: Option<&'h Url>,
    /// Original document URL — fallback when the relative href is
    /// directory-style or `base` is unset.
    pub original_page: Option<&'h Url>,
    /// SSG manifest capture (raw `src` value).  Mirrors
    /// `Page::new_page_streaming` semantics: caller resolves the
    /// stored value via `convert_abs_path` after rewriter ends.
    /// Selector: `script` (matches all script tags, filters in handler).
    pub ssg_raw_src_cell: Option<&'h tokio::sync::OnceCell<String>>,
    /// SSG manifest capture (pre-resolved absolute path).  Mirrors
    /// `Page::links_stream_base_ssg` semantics.  Selector: `script[src]`
    /// (pre-filtered).  Resolution happens inside the closure via
    /// `convert_abs_path` against `base`; if `base` is `None` the
    /// closure no-ops, identical to the legacy site at page.rs:5282.
    pub ssg_resolved_path_cell: Option<&'h tokio::sync::OnceCell<String>>,
    /// Page is `*.xml` — pick the XML link selector.  No effect when
    /// `full_resources = true` (full_resources uses a unified
    /// anchor+script+link selector regardless of doc type).
    pub xml_file: bool,
    /// Capture every `a[href]`, `script[src]`, `link[href]` instead
    /// of just `<a>`.  Mirrors the `r_settings.full_resources` branch.
    pub full_resources: bool,
    /// Skip the link-selector handler entirely.  Metadata, base href,
    /// and SSG handlers still install.  Used for single-page crawls
    /// where the caller only wants page-level data.
    pub skip_links: bool,
}

/// Build the canonical link+metadata handler vector.  Single source of
/// truth for the lol_html element handlers used by every link
/// extraction path (HTTP streaming, in-memory, disk-spooled, SSG, and
/// — once Steps 2-4 land — the Chrome streaming pump).
///
/// Caller is responsible for:
///   - feeding bytes to the rewriter (`rewriter.write(chunk)`),
///   - calling `rewriter.end()` on success,
///   - draining `links_pages` into `self.page_links` after `end()`,
///   - building `Metadata` from `meta_title`/`meta_description`/`meta_og_image` after `end()`,
///   - resolving `ssg_raw_src_cell` via `convert_abs_path` after `end()` (the resolved-path cell mode does this inline).
pub(crate) fn build_link_extract_handlers<'h, A>(
    ctx: LinkExtractCtx<'h, A>,
    meta_title: &'h mut Option<CompactString>,
    meta_description: &'h mut Option<CompactString>,
    meta_og_image: &'h mut Option<CompactString>,
) -> Vec<(
    std::borrow::Cow<'static, lol_html::Selector>,
    lol_html::send::ElementContentHandlers<'h>,
)>
where
    A: PartialEq
        + Eq
        + Sync
        + Send
        + Clone
        + Default
        + std::hash::Hash
        + From<String>
        + for<'a> From<&'a str>,
{
    let LinkExtractCtx {
        selectors,
        external_domains_caseless,
        map,
        links_pages,
        base_input_url,
        base,
        original_page,
        ssg_raw_src_cell,
        ssg_resolved_path_cell,
        xml_file,
        full_resources,
        skip_links,
    } = ctx;

    // Borrow projections from selectors — derived once so each closure
    // doesn't re-walk the tuple at every match.
    let parent_host = &selectors.1[0];
    let parent_host_scheme = &selectors.1[1];
    let base_input_domain = &selectors.2;
    let sub_matcher = &selectors.0;

    let mut handlers = Vec::with_capacity(
        3 /* metadata */
            + 1 /* base element */
            + (!skip_links) as usize
            + ssg_raw_src_cell.is_some() as usize
            + ssg_resolved_path_cell.is_some() as usize,
    );

    // 1. Metadata: title / meta[name=description] / meta[property=og:image].
    handlers.extend(metadata_handlers(
        meta_title,
        meta_description,
        meta_og_image,
    ));

    // 2. <base href> capture — runs first because well-formed docs
    // place `<base>` in `<head>` before any `<a>`, so the link
    // handler's `base_input_url.initialized()` check sees the value.
    handlers.push(element_precompiled!(
        compiled_base_element_selector(),
        move |el| {
            if let Some(href) = el.get_attribute("href") {
                if let Ok(parsed_base) = Url::parse(&href) {
                    let _ = base_input_url.set(parsed_base);
                }
            }
            Ok(())
        }
    ));

    // 3. Link handler — full_resources unifies a/script/link, otherwise
    //    pick the precompiled HTML or XML anchor selector.
    if !skip_links {
        if full_resources {
            handlers.push(lol_html::element!(
                "a[href]:not([aria-hidden=\"true\"]),script[src],link[href]",
                move |el| {
                    let tag_name = el.tag_name();
                    let attribute = if tag_name == "script" { "src" } else { "href" };

                    if let Some(href) = el.get_attribute(attribute) {
                        let b = if relative_directory_url(&href) || base.is_none() {
                            original_page
                        } else {
                            base
                        };
                        let b = if base_input_url.initialized() {
                            base_input_url.get()
                        } else {
                            b
                        };

                        push_link(
                            &b,
                            &href,
                            map,
                            sub_matcher,
                            parent_host,
                            parent_host_scheme,
                            base_input_domain,
                            sub_matcher,
                            external_domains_caseless,
                            links_pages,
                        );
                    }

                    Ok(())
                }
            ));
        } else {
            handlers.push(element_precompiled!(
                if xml_file {
                    compiled_xml_selector()
                } else {
                    compiled_selector()
                },
                move |el| {
                    if let Some(href) = el.get_attribute("href") {
                        let b = if relative_directory_url(&href) || base.is_none() {
                            original_page
                        } else {
                            base
                        };
                        let b = if base_input_url.initialized() {
                            base_input_url.get()
                        } else {
                            b
                        };

                        push_link(
                            &b,
                            &href,
                            map,
                            sub_matcher,
                            parent_host,
                            parent_host_scheme,
                            base_input_domain,
                            sub_matcher,
                            external_domains_caseless,
                            links_pages,
                        );
                    }
                    Ok(())
                }
            ));
        }
    }

    // 4a. SSG manifest — raw-src mode (caller resolves later).
    //     Mirrors `Page::new_page_streaming` (page.rs:2754) which uses
    //     the bare `script` selector and filters inside the closure.
    if let Some(cell) = ssg_raw_src_cell {
        handlers.push(lol_html::element!("script", move |el| {
            if let Some(build_path) = el.get_attribute("src") {
                if build_path.starts_with("/_next/static/")
                    && build_path.ends_with("/_ssgManifest.js")
                {
                    // `get_attribute` returns an owned `String`; move
                    // straight into the cell instead of copying via
                    // `to_string()`.
                    let _ = cell.set(build_path);
                }
            }
            Ok(())
        }));
    }

    // 4b. SSG manifest — resolved-path mode.  Mirrors
    //     `Page::links_stream_base_ssg` (page.rs:5277) which uses the
    //     `script[src]` pre-filtered selector and resolves inline via
    //     `convert_abs_path`.  No-ops when `base` is `None` (matches
    //     the legacy `if let Some(b) = base.map(...)` guard).
    if let Some(cell) = ssg_resolved_path_cell {
        handlers.push(lol_html::element!("script[src]", move |el| {
            if let Some(source) = el.get_attribute("src") {
                if source.starts_with("/_next/static/") && source.ends_with("/_ssgManifest.js") {
                    if let Some(b) = base {
                        let _ = cell.set(convert_abs_path(b, &source).to_string());
                    }
                }
            }
            Ok(())
        }));
    }

    handlers
}

impl Page {
    /// Whether the page needs a retry based on `should_retry`, a retryable status code,
    /// a truncated response (upstream stream ended prematurely), or a proxy-retryable
    /// 401 (when `proxy_configured` is set, proxy rotation may resolve the auth failure).
    #[inline]
    pub fn needs_retry(&self) -> bool {
        self.should_retry
            || self.content_truncated
            || is_retryable_status(self.status_code)
            || (self.proxy_configured && self.status_code == StatusCode::UNAUTHORIZED)
    }

    /// Instantiate a new page and gather the html repro of standard fetch_page_html.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page(url: &str, client: &Client) -> Self {
        let page_resource: PageResponse = crate::utils::fetch_page_html_raw(url, client).await;

        build(url, page_resource)
    }

    /// Same as [`new_page`] but arms the HTTP first-byte watchdog. When
    /// `first_byte_timeout` is `Some`, each `req.send()` is wrapped in
    /// `tokio::time::timeout(base + rand(0..jitter))`. On timeout the
    /// in-flight connect / header future is dropped and a synthetic
    /// `524 GATEWAY_TIMEOUT` PageResponse is built so the caller's
    /// retry path rotates the proxy. Pass `(None, None)` for behavior
    /// identical to `new_page`.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_with_watchdog(
        url: &str,
        client: &Client,
        first_byte_timeout: Option<std::time::Duration>,
        first_byte_jitter: Option<std::time::Duration>,
    ) -> Self {
        let page_resource: PageResponse = crate::utils::fetch_page_html_raw_with_watchdog(
            url,
            client,
            first_byte_timeout,
            first_byte_jitter,
        )
        .await;

        build(url, page_resource)
    }

    /// Auto-armed variant: consults `Configuration::auto_http_first_byte_args`
    /// and arms the watchdog only when the gate fires:
    ///
    /// * `balance` feature enabled, AND
    /// * `config.proxies` has ≥ 2 entries usable for HTTP
    ///   (`ignore != ProxyIgnore::Http`).
    ///
    /// Without those conditions, behavior is identical to
    /// [`new_page`] — no point arming the watchdog when there's no
    /// alternate proxy to rotate to. The configured
    /// `http_first_byte_timeout` + `_jitter` on `Configuration` are
    /// only consumed when the gate fires.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_auto_watchdog(
        url: &str,
        client: &Client,
        config: &crate::configuration::Configuration,
    ) -> Self {
        let (base, jitter) = config.auto_http_first_byte_args();
        Self::new_page_with_watchdog(url, client, base, jitter).await
    }

    /// Instantiate a new page using cache options when available.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_with_cache(
        url: &str,
        client: &Client,
        cache_options: Option<CacheOptions>,
        cache_policy: &Option<BasicCachePolicy>,
        cache_namespace: Option<&str>,
    ) -> Self {
        let page_resource: PageResponse = crate::utils::fetch_page_html_raw_cached(
            url,
            client,
            cache_options,
            cache_policy,
            cache_namespace,
        )
        .await;

        build(url, page_resource)
    }

    /// Create a new page from WebDriver content.
    #[cfg(feature = "webdriver")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub fn new_webdriver(url: &str, html: String, status_code: StatusCode) -> Self {
        let mut page = Page::default();
        page.html = Some(bytes::Bytes::from(html.into_bytes()));
        page.url = url.into();
        page.status_code = status_code;
        page
    }

    /// Create a new page from WebDriver with full response.
    #[cfg(feature = "webdriver")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_webdriver(
        url: &str,
        driver: &std::sync::Arc<thirtyfour::WebDriver>,
        timeout: Option<std::time::Duration>,
    ) -> Self {
        use crate::features::webdriver::{attempt_navigation, get_current_url, get_page_content};

        // Navigate to the URL
        if let Err(e) = attempt_navigation(url, driver, &timeout).await {
            log::error!("WebDriver navigation failed: {:?}", e);
            let mut page = Page::default();
            page.url = url.into();
            page.status_code = *UNKNOWN_STATUS_ERROR;
            #[cfg(not(feature = "page_error_status_details"))]
            {
                page.error_status = Some(format!("WebDriver navigation failed: {:?}", e));
            }
            return page;
        }

        // Get current URL (may have redirected)
        let final_url = get_current_url(driver).await.ok();

        // Get page content
        match get_page_content(driver).await {
            Ok(content) => {
                let mut page = Page::default();
                page.html = Some(bytes::Bytes::from(content.into_bytes()));
                page.url = url.into();
                page.status_code = StatusCode::OK;
                page.final_redirect_destination = final_url;
                page
            }
            Err(e) => {
                log::error!("Failed to get WebDriver page content: {:?}", e);
                let mut page = Page::default();
                page.url = url.into();
                page.status_code = *UNKNOWN_STATUS_ERROR;
                #[cfg(not(feature = "page_error_status_details"))]
                {
                    page.error_status = Some(format!("Failed to get page content: {:?}", e));
                }
                page
            }
        }
    }

    /// Create a new page from WebDriver with full response and automation support.
    #[cfg(all(feature = "webdriver", feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_webdriver_full(
        url: &str,
        driver: &std::sync::Arc<thirtyfour::WebDriver>,
        timeout: Option<std::time::Duration>,
        wait_for: &Option<crate::configuration::WaitFor>,
        execution_scripts: &Option<crate::features::chrome_common::ExecutionScripts>,
        automation_scripts: &Option<crate::features::chrome_common::AutomationScripts>,
    ) -> Self {
        use crate::features::webdriver::{
            attempt_navigation, get_current_url, get_page_content, run_execution_scripts,
            run_url_automation_scripts,
        };

        // Navigate to the URL
        if let Err(e) = attempt_navigation(url, driver, &timeout).await {
            log::error!("WebDriver navigation failed: {:?}", e);
            let mut page = Page::default();
            page.url = url.into();
            page.status_code = *UNKNOWN_STATUS_ERROR;
            #[cfg(not(feature = "page_error_status_details"))]
            {
                page.error_status = Some(format!("WebDriver navigation failed: {:?}", e));
            }
            return page;
        }

        // Run execution scripts for this URL
        run_execution_scripts(driver, url, execution_scripts).await;

        // Run automation scripts for this URL
        run_url_automation_scripts(driver, url, automation_scripts).await;

        // Handle wait_for configuration
        if let Some(wait_config) = wait_for {
            // Handle delay wait
            if let Some(ref delay) = wait_config.delay {
                if let Some(timeout_duration) = delay.timeout {
                    tokio::time::sleep(timeout_duration).await;
                }
            }
            // Handle selector wait
            if let Some(ref selector_wait) = wait_config.selector {
                let wait_timeout = selector_wait
                    .timeout
                    .unwrap_or(std::time::Duration::from_secs(30));
                let _ = crate::features::webdriver::wait_for_element(
                    driver,
                    &selector_wait.selector,
                    wait_timeout,
                )
                .await;
            }
            // Handle idle network wait (approximate with delay)
            if let Some(ref idle) = wait_config.idle_network {
                let wait_time = idle.timeout.unwrap_or(std::time::Duration::from_secs(5));
                tokio::time::sleep(wait_time).await;
            }
        }

        // Get current URL (may have redirected)
        let final_url = get_current_url(driver).await.ok();

        // Get page content
        match get_page_content(driver).await {
            Ok(content) => {
                let mut page = Page::default();
                page.html = Some(bytes::Bytes::from(content.into_bytes()));
                page.url = url.into();
                page.status_code = StatusCode::OK;
                page.final_redirect_destination = final_url;
                page
            }
            Err(e) => {
                log::error!("Failed to get WebDriver page content: {:?}", e);
                let mut page = Page::default();
                page.url = url.into();
                page.status_code = *UNKNOWN_STATUS_ERROR;
                #[cfg(not(feature = "page_error_status_details"))]
                {
                    page.error_status = Some(format!("Failed to get page content: {:?}", e));
                }
                page
            }
        }
    }

    /// New page with rewriter
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_streaming<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + std::hash::Hash
            + From<String>
            + for<'a> From<&'a str>,
    >(
        url: &str,
        client: &Client,
        only_html: bool,
        selectors: &mut RelativeSelectors,
        external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
        r_settings: &PageLinkBuildSettings,
        map: &mut hashbrown::HashSet<A>,
        ssg_map: Option<&mut hashbrown::HashSet<A>>,
        prior_domain: &Option<Box<Url>>,
        domain_parsed: &mut Option<Box<Url>>,
        links_pages: &mut Option<hashbrown::HashSet<A>>,
    ) -> Self {
        use crate::utils::{
            handle_response_bytes, handle_response_bytes_writer, modify_selectors,
            AllowedDomainTypes,
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        let duration = if cfg!(feature = "time") {
            Some(tokio::time::Instant::now())
        } else {
            None
        };

        #[cfg(feature = "balance")]
        crate::utils::vitals::request_start();

        let mut page_response: PageResponse = match client.get(url).send().await {
            Ok(res)
                if crate::utils::valid_parsing_status(&res)
                    && !crate::utils::block_streaming(&res, only_html) =>
            {
                let cell = if r_settings.ssg_build {
                    Some(tokio::sync::OnceCell::new())
                } else {
                    None
                };

                let base_input_url = tokio::sync::OnceCell::new();

                let (encoding, adjust_charset_on_meta_tag) =
                    match get_charset_from_content_type(res.headers()) {
                        Some(h) => (h, false),
                        _ => (AsciiCompatibleEncoding::utf_8(), true),
                    };

                let target_url = res.url().as_str();

                // handle initial redirects
                if ssg_map.is_some() && url != target_url && !exact_url_match(url, target_url) {
                    let mut url = Box::new(CaseInsensitiveString::new(&url));

                    modify_selectors(
                        prior_domain,
                        target_url,
                        domain_parsed,
                        &mut url,
                        selectors,
                        AllowedDomainTypes::new(r_settings.subdomains, r_settings.tld),
                    );
                };

                let base = if domain_parsed.is_none() {
                    prior_domain
                } else {
                    domain_parsed
                };

                let original_page = Url::parse(url).ok();
                let xml_file = target_url.ends_with(".xml");

                // Locals used by the post-rewriter SSG block below for
                // its own `push_link` calls — kept around even after the
                // helper consumes its own copy of the same projections.
                let parent_host = &selectors.1[0];
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2;
                let sub_matcher = &selectors.0;

                let element_content_handlers = build_link_extract_handlers(
                    LinkExtractCtx {
                        selectors,
                        external_domains_caseless,
                        map,
                        links_pages,
                        base_input_url: &base_input_url,
                        base: base.as_deref(),
                        original_page: original_page.as_ref(),
                        ssg_raw_src_cell: if r_settings.ssg_build && !r_settings.skip_links {
                            cell.as_ref()
                        } else {
                            None
                        },
                        ssg_resolved_path_cell: None,
                        xml_file,
                        full_resources: r_settings.full_resources,
                        skip_links: r_settings.skip_links,
                    },
                    &mut meta_title,
                    &mut meta_description,
                    &mut meta_og_image,
                );

                let settings = lol_html::send::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag,
                    encoding,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter = lol_html::send::HtmlRewriter::new(settings, |_c: &[u8]| {});

                let mut collected_bytes = match res.content_length() {
                    Some(cap) if cap > MAX_CONTENT_LENGTH => {
                        log::warn!("{url} Content-Length {cap} exceeds 2 GB limit, rejecting");
                        Vec::new()
                    }
                    Some(cap) if cap > 0 => Vec::with_capacity((cap as usize).min(MAX_PREALLOC)),
                    _ => Vec::with_capacity(MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE),
                };

                let mut response = handle_response_bytes_writer(
                    res,
                    url,
                    only_html,
                    &mut rewriter,
                    &mut collected_bytes,
                )
                .await;

                let rewrite_error = response.1;

                if !rewrite_error {
                    let _ = rewriter.end();
                }

                if r_settings.normalize {
                    response.0.signature = Some(hash_html(&collected_bytes).await);
                }

                response.0.content = if collected_bytes.is_empty() {
                    None
                } else {
                    Some(collected_bytes)
                };

                if r_settings.ssg_build {
                    if let Some(ssg_map) = ssg_map {
                        if let Some(cell) = &cell {
                            if let Some(source) = cell.get() {
                                if let Some(url_base) = &base {
                                    let build_ssg_path = convert_abs_path(url_base, source);
                                    let build_page =
                                        Page::new_page(build_ssg_path.as_str(), client).await;

                                    for cap in
                                        SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8())
                                    {
                                        if let Some(matched) = cap.get(1) {
                                            let href = auto_encode_bytes(matched.as_bytes())
                                                .replace(r#"\u002F"#, "/");

                                            let last_segment =
                                                crate::utils::get_last_segment(&href);

                                            // we can pass in a static map of the dynamic SSG routes pre-hand, custom API endpoint to seed, or etc later.
                                            if !(last_segment.starts_with("[")
                                                && last_segment.ends_with("]"))
                                            {
                                                let base = if relative_directory_url(&href)
                                                    || base.is_none()
                                                {
                                                    original_page.as_ref()
                                                } else {
                                                    base.as_deref()
                                                };
                                                let base = if base_input_url.initialized() {
                                                    base_input_url.get()
                                                } else {
                                                    base
                                                };
                                                push_link(
                                                    &base,
                                                    &href,
                                                    ssg_map,
                                                    &selectors.0,
                                                    parent_host,
                                                    parent_host_scheme,
                                                    base_input_domain,
                                                    sub_matcher,
                                                    external_domains_caseless,
                                                    &mut None,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                response.0
            }
            Ok(res) => {
                let pr = handle_response_bytes(res, url, only_html).await;
                if pr.content_truncated {
                    log::warn!("Response truncated for {url}, retrying once");
                    match client.get(url).send().await {
                        Ok(res2) => handle_response_bytes(res2, url, only_html).await,
                        Err(_) => pr,
                    }
                } else {
                    pr
                }
            }
            Err(err) => {
                log::info!("error fetching {}", url);

                let mut page_response = PageResponse::default();

                if let Some(status_code) = err.status() {
                    page_response.status_code = status_code;
                } else {
                    page_response.status_code = crate::page::get_error_http_status_code(&err);
                }

                page_response.error_for_status = Some(Err(err));

                #[cfg(feature = "balance")]
                crate::utils::vitals::request_error();

                page_response
            }
        };

        #[cfg(feature = "balance")]
        crate::utils::vitals::request_end();

        let valid_meta = meta_title.is_some()
            || meta_description.is_some()
            || meta_og_image.is_some()
            || metadata.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                // Preserve automation results from existing metadata if present
                set_metadata(&mut metadata, &mut metadata_inner);
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                page_response.metadata = metadata;
            }
        }

        crate::utils::set_page_response_duration(&mut page_response, duration);

        build(url, page_response)
    }

    /// Instantiate a new page and gather the html repro of standard fetch_page_html only gathering resources to crawl.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new_page_only_html(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html_raw_only_html(url, client).await;
        build(url, page_resource)
    }

    /// Instantiate a new page and gather the html.
    #[cfg(all(not(feature = "decentralized"), not(feature = "chrome")))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new(url: &str, client: &Client) -> Self {
        let page_resource = crate::utils::fetch_page_html(url, client).await;
        build(url, page_resource)
    }

    /// Instantiate a new page and gather the links from input bytes.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    #[cfg(feature = "cmd")]
    pub async fn new_page_streaming_from_bytes<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + std::hash::Hash
            + From<String>
            + for<'a> From<&'a str>,
    >(
        url: &str,
        input_bytes: &[u8],
        selectors: &mut RelativeSelectors,
        external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
        r_settings: &PageLinkBuildSettings,
        map: &mut hashbrown::HashSet<A>,
        ssg_map: Option<&mut hashbrown::HashSet<A>>,
        prior_domain: &Option<Box<Url>>,
        domain_parsed: &mut Option<Box<Url>>,
        links_pages: &mut Option<hashbrown::HashSet<A>>,
    ) -> Self {
        use crate::utils::{modify_selectors, AllowedDomainTypes};

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        let duration = if cfg!(feature = "time") {
            Some(tokio::time::Instant::now())
        } else {
            None
        };

        let encoding = AsciiCompatibleEncoding::utf_8();
        let adjust_charset_on_meta_tag = true;

        let base_input_url = tokio::sync::OnceCell::new();

        let original_page = Url::parse(url).ok();

        if ssg_map.is_some() {
            let mut ci_url = Box::new(CaseInsensitiveString::new(url));
            modify_selectors(
                prior_domain,
                url,
                domain_parsed,
                &mut ci_url,
                selectors,
                AllowedDomainTypes::new(r_settings.subdomains, r_settings.tld),
            );
        }

        let base = if domain_parsed.is_none() {
            prior_domain
        } else {
            domain_parsed
        };

        let xml_file = url.ends_with(".xml");

        let element_content_handlers = build_link_extract_handlers(
            LinkExtractCtx {
                selectors,
                external_domains_caseless,
                map,
                links_pages,
                base_input_url: &base_input_url,
                base: base.as_deref(),
                original_page: original_page.as_ref(),
                // Preserves prior behavior: this function never installed
                // an SSG handler (capacity hint at the legacy site was
                // off-by-one but no handler was pushed).
                ssg_raw_src_cell: None,
                ssg_resolved_path_cell: None,
                xml_file,
                full_resources: r_settings.full_resources,
                skip_links: false,
            },
            &mut meta_title,
            &mut meta_description,
            &mut meta_og_image,
        );

        let settings = lol_html::send::Settings {
            element_content_handlers,
            adjust_charset_on_meta_tag,
            encoding,
            ..lol_html::send::Settings::new_for_handler_types()
        };

        let mut collected_bytes: Vec<u8> = match input_bytes.len() {
            n if n >= MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE => Vec::with_capacity(n),
            n if n > 0 => Vec::with_capacity(n),
            _ => Vec::with_capacity(MAX_PRE_ALLOCATED_HTML_PAGE_SIZE_USIZE),
        };

        let mut rewriter = lol_html::send::HtmlRewriter::new(settings, |c: &[u8]| {
            collected_bytes.extend_from_slice(c);
        });

        let _ = rewriter.write(input_bytes);
        let _ = rewriter.end();

        let mut page_response = PageResponse::default();
        page_response.status_code = StatusCode::OK;

        if r_settings.normalize {
            page_response.signature = Some(hash_html(&collected_bytes).await);
        }

        if !collected_bytes.is_empty() {
            page_response.content = Some(collected_bytes);
        }

        let valid_meta = meta_title.is_some()
            || meta_description.is_some()
            || meta_og_image.is_some()
            || metadata.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                // Preserve automation results from existing metadata if present
                set_metadata(&mut metadata, &mut metadata_inner);
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                page_response.metadata = metadata;
            }
        }

        crate::utils::set_page_response_duration(&mut page_response, duration);

        build(url, page_response)
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Instantiate a new page and gather the html.
    pub(crate) async fn new_base<'h>(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        page_set: bool,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
        cache_options: Option<CacheOptions>,
        seeded_resource: Option<String>,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
        cache_namespace: Option<&str>,
        params: &crate::utils::ChromeFetchParams<'_>,
        extract: Option<&mut ChromeStreamingExtractor<'h>>,
    ) -> Self {
        let page_resource = if seeded_resource.is_some() {
            crate::utils::fetch_page_html_seeded(
                url,
                client,
                page,
                page_set,
                referrer,
                max_page_bytes,
                cache_options,
                seeded_resource,
                jar,
                cache_namespace,
                params,
                extract,
            )
            .await
        } else {
            #[cfg(feature = "fs")]
            {
                crate::utils::fetch_page_html(
                    url,
                    client,
                    page,
                    page_set,
                    referrer,
                    max_page_bytes,
                    cache_options,
                    #[cfg(feature = "cookies")]
                    jar,
                    cache_namespace,
                    params,
                    extract,
                )
                .await
            }
            #[cfg(not(feature = "fs"))]
            {
                let _ = jar;
                crate::utils::fetch_page_html(
                    url,
                    client,
                    page,
                    page_set,
                    referrer,
                    max_page_bytes,
                    cache_options,
                    cache_namespace,
                    params,
                    extract,
                )
                .await
            }
        };
        let mut p = build(url, page_resource);

        // store the chrome page to perform actions like screenshots etc.
        if cfg!(feature = "chrome_store_page") {
            p.chrome_page = Some(page.clone());
        }

        p
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Instantiate a new page and gather the html.
    pub async fn new(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        page_set: bool,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
        cache_options: Option<CacheOptions>,
        cache_namespace: Option<&str>,
        params: &crate::utils::ChromeFetchParams<'_>,
    ) -> Self {
        Self::new_base(
            url,
            client,
            page,
            page_set,
            referrer,
            max_page_bytes,
            cache_options,
            None,
            None,
            cache_namespace,
            params,
            None,
        )
        .await
    }

    /// Streaming-extraction variant of [`Page::new`].  Used by
    /// `chrome_page_fetch!` to fold link/metadata extraction into the
    /// chrome chunk pump — a single lol_html pass over the response
    /// bytes instead of the legacy `fetch + page.links()` two-pass.
    ///
    /// Caller pre-allocates the `links` and `links_pages` sets and
    /// passes them by `&mut`.  On success (`extract_succeeded = true`)
    /// the sets are populated and the post-process layer skips the
    /// redundant `page.links()` walk.  On failure (CDP error
    /// mid-stream, rewriter write error) the post-process layer falls
    /// back to the legacy second-pass extraction over the assembled
    /// body — keeping behavior byte-identical to prior releases.
    ///
    /// Metadata (`title` / `description` / `og:image`) is captured in
    /// the same handler vector and copied onto `Page::metadata` after
    /// the rewriter ends, mirroring the legacy `links_stream_base`
    /// post-stream block exactly.
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub(crate) async fn new_streaming<A>(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        page_set: bool,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
        cache_options: Option<CacheOptions>,
        cache_namespace: Option<&str>,
        params: &crate::utils::ChromeFetchParams<'_>,
        selectors: &RelativeSelectors,
        external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
        links: &mut HashSet<A>,
        links_pages: &mut Option<HashSet<A>>,
        full_resources: bool,
        skip_links: bool,
        ssg_enabled: bool,
    ) -> (Self, bool)
    where
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    {
        let parsed_target = Url::parse(url).ok();
        let xml_file = url.ends_with(".xml");
        let base_input_url = tokio::sync::OnceCell::new();
        // Pre-bytes asset gate: URLs ending in known binary extensions
        // (.png/.jpg/.pdf/.zip/etc.) skip the streaming pump entirely.
        // The chrome fetch still runs because callers (e.g.
        // `chrome_page_post_process!`) need the bytes for headers /
        // anti-bot / signature, but `lol_html` doesn't see them and
        // `extract_succeeded` stays false so the post-process layer's
        // `is_binary_spool_aware` check returns Default for the link
        // set — same as the legacy second-pass behavior.
        let asset_url = is_asset_url(url);
        let ssg_cell = if ssg_enabled && !skip_links && !xml_file && !asset_url {
            Some(tokio::sync::OnceCell::new())
        } else {
            None
        };

        let mut meta_title: Option<CompactString> = None;
        let mut meta_description: Option<CompactString> = None;
        let mut meta_og_image: Option<CompactString> = None;

        let (page_out, mut extract_succeeded) = if asset_url {
            // Skip the rewriter setup entirely. `new_base` still runs
            // the chrome fetch but with `extract = None`, so no
            // streaming work happens.
            let p = Self::new_base(
                url,
                client,
                page,
                page_set,
                referrer,
                max_page_bytes,
                cache_options,
                None,
                None,
                cache_namespace,
                params,
                None,
            )
            .await;
            (p, false)
        } else {
            let handlers = build_link_extract_handlers(
                LinkExtractCtx {
                    selectors,
                    external_domains_caseless,
                    map: links,
                    links_pages,
                    base_input_url: &base_input_url,
                    base: parsed_target.as_ref(),
                    original_page: parsed_target.as_ref(),
                    ssg_raw_src_cell: None,
                    ssg_resolved_path_cell: ssg_cell.as_ref(),
                    xml_file,
                    full_resources,
                    // When the caller already knows it doesn't need
                    // links (single-page mode without return_page_links),
                    // skip the link handler entirely — metadata + base
                    // capture still install so meta_title/description/
                    // og_image stay populated for downstream consumers.
                    skip_links,
                },
                &mut meta_title,
                &mut meta_description,
                &mut meta_og_image,
            );

            let mut extract = ChromeStreamingExtractor::new(handlers, None, true);
            let p = Self::new_base(
                url,
                client,
                page,
                page_set,
                referrer,
                max_page_bytes,
                cache_options,
                None,
                None,
                cache_namespace,
                params,
                Some(&mut extract),
            )
            .await;
            let succeeded = extract.end();
            (p, succeeded)
        };

        let mut p = page_out;

        // SSG manifest capture (post-stream).  Mirrors
        // `Page::links_stream_base_ssg` exactly — fetches the build
        // manifest URL captured by the rewriter and appends every quoted
        // path to the link set, modulo selector / external-domain rules.
        if extract_succeeded && !skip_links {
            if let Some(cell) = ssg_cell.as_ref() {
                if let Some(build_ssg_path) = cell.get() {
                    if !build_ssg_path.is_empty() {
                        let build_page = Self::new_page(build_ssg_path, client).await;
                        let parent_host = &selectors.1[0];
                        let parent_host_scheme = &selectors.1[1];
                        let base_input_domain = &selectors.2;
                        let sub_matcher = &selectors.0;
                        let ssg_base = if base_input_url.initialized() {
                            base_input_url.get()
                        } else {
                            parsed_target.as_ref()
                        };

                        for cap in SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8()) {
                            if let Some(matched) = cap.get(1) {
                                let href =
                                    auto_encode_bytes(matched.as_bytes()).replace(r#"\u002F"#, "/");
                                let last_segment = crate::utils::get_last_segment(&href);

                                if !(last_segment.starts_with("[") && last_segment.ends_with("]")) {
                                    let resolved_base =
                                        if relative_directory_url(&href) || ssg_base.is_none() {
                                            parsed_target.as_ref()
                                        } else {
                                            ssg_base
                                        };

                                    push_link(
                                        &resolved_base,
                                        &href,
                                        links,
                                        sub_matcher,
                                        parent_host,
                                        parent_host_scheme,
                                        base_input_domain,
                                        sub_matcher,
                                        external_domains_caseless,
                                        &mut None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // XML extraction — chrome's xml_target carve-out at
        // `fetch_page_html_chrome_base` skips the lol_html pump entirely,
        // so streaming reports failure and `page.is_xml` (set from the
        // `<?xml` content prefix) routes us through `quick_xml` here.
        // Mirrors the legacy `Page::links_stream_xml_links_stream_base`
        // call previously made by `crawl_establish` after fetch.
        if !extract_succeeded && !skip_links && p.is_xml {
            if let Some(html_bytes) = p.html.take() {
                p.links_stream_xml_links_stream_base(selectors, &html_bytes, links, &None)
                    .await;
                p.html = Some(html_bytes);
                extract_succeeded = true;
            }
        }

        // Behavior parity with the legacy `chrome_page_post_process!`
        // path: `set_url_parsed_direct` uses `final_redirect_destination`
        // when present.  The streaming pass resolved relative links
        // against the *requested* `url`, so a redirect to a different
        // origin would skew base-URL resolution. Invalidate when the
        // recorded redirect doesn't match the requested URL — the
        // post-process layer will re-extract via `page.links()` over the
        // assembled body using the correct base.
        if extract_succeeded {
            if let Some(redirect) = p.final_redirect_destination.as_deref() {
                if redirect != url {
                    extract_succeeded = false;
                }
            }
        }

        if extract_succeeded {
            let valid_meta =
                meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

            if valid_meta {
                let mut metadata_inner = Metadata::default();
                metadata_inner.title = meta_title;
                metadata_inner.description = meta_description;
                metadata_inner.image = meta_og_image;

                if metadata_inner.exist() {
                    set_metadata(&mut p.metadata, &mut metadata_inner);
                    p.metadata.replace(Box::new(metadata_inner));
                }
            }

            update_link_capacity_hint(links.len());
        }

        (p, extract_succeeded)
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Instantiate a new page and gather the html seeded.
    pub async fn new_seeded(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        page_set: bool,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
        cache_options: Option<CacheOptions>,
        seeded_resource: Option<String>,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
        cache_namespace: Option<&str>,
        params: &crate::utils::ChromeFetchParams<'_>,
    ) -> Self {
        Self::new_base(
            url,
            client,
            page,
            page_set,
            referrer,
            max_page_bytes,
            cache_options,
            seeded_resource,
            jar,
            cache_namespace,
            params,
            None,
        )
        .await
    }

    /// Streaming-extraction variant of [`Page::new_seeded`].
    /// See [`Page::new_streaming`] for the contract.
    ///
    /// Currently unused — `crawl_establish` (the seeded-resource entry
    /// point) calls [`Page::links_ssg`] which performs cross-domain
    /// manifest fetches that the streaming handler vector does not yet
    /// emulate.  Kept as an `pub(crate)` API so a future refactor can
    /// land the streaming optimization there too.
    #[allow(dead_code)]
    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub(crate) async fn new_seeded_streaming<A>(
        url: &str,
        client: &Client,
        page: &chromiumoxide::Page,
        page_set: bool,
        referrer: Option<String>,
        max_page_bytes: Option<f64>,
        cache_options: Option<CacheOptions>,
        seeded_resource: Option<String>,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
        cache_namespace: Option<&str>,
        params: &crate::utils::ChromeFetchParams<'_>,
        selectors: &RelativeSelectors,
        external_domains_caseless: &Arc<HashSet<CaseInsensitiveString>>,
        links: &mut HashSet<A>,
        links_pages: &mut Option<HashSet<A>>,
        full_resources: bool,
        skip_links: bool,
        ssg_enabled: bool,
    ) -> (Self, bool)
    where
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    {
        let parsed_target = Url::parse(url).ok();
        let xml_file = url.ends_with(".xml");
        let base_input_url = tokio::sync::OnceCell::new();
        // Pre-bytes asset gate — see `Page::new_streaming` for rationale.
        let asset_url = is_asset_url(url);
        let ssg_cell = if ssg_enabled && !skip_links && !xml_file && !asset_url {
            Some(tokio::sync::OnceCell::new())
        } else {
            None
        };

        let mut meta_title: Option<CompactString> = None;
        let mut meta_description: Option<CompactString> = None;
        let mut meta_og_image: Option<CompactString> = None;

        let (page_out, mut extract_succeeded) = if asset_url {
            // Skip the rewriter setup entirely — see `Page::new_streaming`.
            let p = Self::new_base(
                url,
                client,
                page,
                page_set,
                referrer,
                max_page_bytes,
                cache_options,
                seeded_resource,
                jar,
                cache_namespace,
                params,
                None,
            )
            .await;
            (p, false)
        } else {
            let handlers = build_link_extract_handlers(
                LinkExtractCtx {
                    selectors,
                    external_domains_caseless,
                    map: links,
                    links_pages,
                    base_input_url: &base_input_url,
                    base: parsed_target.as_ref(),
                    original_page: parsed_target.as_ref(),
                    ssg_raw_src_cell: None,
                    ssg_resolved_path_cell: ssg_cell.as_ref(),
                    xml_file,
                    full_resources,
                    skip_links,
                },
                &mut meta_title,
                &mut meta_description,
                &mut meta_og_image,
            );

            let mut extract = ChromeStreamingExtractor::new(handlers, None, true);
            let p = Self::new_base(
                url,
                client,
                page,
                page_set,
                referrer,
                max_page_bytes,
                cache_options,
                seeded_resource,
                jar,
                cache_namespace,
                params,
                Some(&mut extract),
            )
            .await;
            let succeeded = extract.end();
            (p, succeeded)
        };

        let mut p = page_out;

        // See `Page::new_streaming` — same redirect-base parity guard.
        if extract_succeeded {
            if let Some(redirect) = p.final_redirect_destination.as_deref() {
                if redirect != url {
                    extract_succeeded = false;
                }
            }
        }

        // SSG manifest capture (post-stream).  Mirrors
        // `Page::new_streaming` exactly.
        if extract_succeeded && !skip_links {
            if let Some(cell) = ssg_cell.as_ref() {
                if let Some(build_ssg_path) = cell.get() {
                    if !build_ssg_path.is_empty() {
                        let build_page = Self::new_page(build_ssg_path, client).await;
                        let parent_host = &selectors.1[0];
                        let parent_host_scheme = &selectors.1[1];
                        let base_input_domain = &selectors.2;
                        let sub_matcher = &selectors.0;
                        let ssg_base = if base_input_url.initialized() {
                            base_input_url.get()
                        } else {
                            parsed_target.as_ref()
                        };

                        for cap in SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8()) {
                            if let Some(matched) = cap.get(1) {
                                let href =
                                    auto_encode_bytes(matched.as_bytes()).replace(r#"\u002F"#, "/");
                                let last_segment = crate::utils::get_last_segment(&href);

                                if !(last_segment.starts_with("[") && last_segment.ends_with("]")) {
                                    let resolved_base =
                                        if relative_directory_url(&href) || ssg_base.is_none() {
                                            parsed_target.as_ref()
                                        } else {
                                            ssg_base
                                        };

                                    push_link(
                                        &resolved_base,
                                        &href,
                                        links,
                                        sub_matcher,
                                        parent_host,
                                        parent_host_scheme,
                                        base_input_domain,
                                        sub_matcher,
                                        external_domains_caseless,
                                        &mut None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // XML extraction — same logic as `Page::new_streaming`.
        if !extract_succeeded && !skip_links && p.is_xml {
            if let Some(html_bytes) = p.html.take() {
                p.links_stream_xml_links_stream_base(selectors, &html_bytes, links, &None)
                    .await;
                p.html = Some(html_bytes);
                extract_succeeded = true;
            }
        }

        if extract_succeeded {
            let valid_meta =
                meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

            if valid_meta {
                let mut metadata_inner = Metadata::default();
                metadata_inner.title = meta_title;
                metadata_inner.description = meta_description;
                metadata_inner.image = meta_og_image;

                if metadata_inner.exist() {
                    set_metadata(&mut p.metadata, &mut metadata_inner);
                    p.metadata.replace(Box::new(metadata_inner));
                }
            }

            update_link_capacity_hint(links.len());
        }

        (p, extract_succeeded)
    }

    /// Instantiate a new page and gather the links.
    #[cfg(all(feature = "decentralized", not(feature = "headers")))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn new(url: &str, client: &Client) -> Self {
        Self::new_links_only(url, client).await
    }

    /// Instantiate a new page and gather the headers and links.
    #[cfg(all(feature = "decentralized", feature = "headers"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    pub async fn new(url: &str, client: &Client) -> Self {
        use crate::serde::Deserialize;

        match crate::utils::fetch_page_and_headers(url, client).await {
            FetchPageResult::Success(headers, page_content) => {
                let links = match page_content {
                    Some(b) => match flexbuffers::Reader::get_root(b.as_slice()) {
                        Ok(buf) => match HashSet::<CaseInsensitiveString>::deserialize(buf) {
                            Ok(link) => link,
                            _ => Default::default(),
                        },
                        _ => Default::default(),
                    },
                    _ => Default::default(),
                };
                Page {
                    html: None,
                    headers: Some(headers),
                    links,
                    ..Default::default()
                }
            }
            FetchPageResult::NoSuccess(headers) => Page {
                headers: Some(headers),
                ..Default::default()
            },
            FetchPageResult::FetchError => Default::default(),
        }
    }

    /// Instantiate a new page and gather the links.
    #[cfg(feature = "decentralized")]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    pub async fn new_links_only(url: &str, client: &Client) -> Self {
        use crate::serde::Deserialize;

        let links = match crate::utils::fetch_page(url, client).await {
            Some(b) => match flexbuffers::Reader::get_root(b.as_slice()) {
                Ok(buf) => match HashSet::<CaseInsensitiveString>::deserialize(buf) {
                    Ok(link) => link,
                    _ => Default::default(),
                },
                _ => Default::default(),
            },
            _ => Default::default(),
        };

        Page {
            html: None,
            links,
            ..Default::default()
        }
    }

    #[cfg(not(all(not(feature = "decentralized"), feature = "chrome")))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved.
    /// The feature flag `chrome_store_page` is required.
    pub async fn screenshot(
        &self,
        _full_page: bool,
        _omit_background: bool,
        _format: crate::configuration::CaptureScreenshotFormat,
        _quality: Option<i64>,
        _output_path: Option<impl AsRef<std::path::Path>>,
        _clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        Default::default()
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved.
    /// The feature flag `chrome_store_page` is required.
    pub async fn take_screenshot(
        page: &Page,
        full_page: bool,
        omit_background: bool,
        format: crate::configuration::CaptureScreenshotFormat,
        quality: Option<i64>,
        output_path: Option<impl AsRef<std::path::Path>>,
        clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        match &page.chrome_page {
            Some(chrome_page) => {
                let format: chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat =
                    chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat::from(
                        format,
                    );

                let screenshot_configs = chromiumoxide::page::ScreenshotParams::builder()
                    .format(format)
                    .full_page(full_page)
                    .omit_background(omit_background);

                let screenshot_configs = match quality {
                    Some(q) => screenshot_configs.quality(q),
                    _ => screenshot_configs,
                };

                let screenshot_configs = match clip {
                    Some(vp) => screenshot_configs.clip(
                        chromiumoxide::cdp::browser_protocol::page::Viewport::from(vp),
                    ),
                    _ => screenshot_configs,
                };

                if output_path.is_none() {
                    match chrome_page.screenshot(screenshot_configs.build()).await {
                        Ok(v) => {
                            log::debug!("took screenshot: {:?}", page.url);
                            v
                        }
                        Err(e) => {
                            log::error!("failed to took screenshot: {:?} - {:?}", e, page.url);
                            Default::default()
                        }
                    }
                } else {
                    let output_path = match output_path {
                        Some(out) => out.as_ref().to_path_buf(),
                        _ => Default::default(),
                    };

                    match chrome_page
                        .save_screenshot(screenshot_configs.build(), &output_path)
                        .await
                    {
                        Ok(v) => {
                            log::debug!("saved screenshot: {:?}", output_path);
                            v
                        }
                        Err(e) => {
                            log::error!("failed to save screenshot: {:?} - {:?}", e, output_path);
                            Default::default()
                        }
                    }
                }
            }
            _ => Default::default(),
        }
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    /// Take a screenshot of the page. If the output path is set to None the screenshot will not be saved. The feature flag `chrome_store_page` is required.
    pub async fn screenshot(
        &self,
        full_page: bool,
        omit_background: bool,
        format: crate::configuration::CaptureScreenshotFormat,
        quality: Option<i64>,
        output_path: Option<impl AsRef<std::path::Path>>,
        clip: Option<crate::configuration::ClipViewport>,
    ) -> Vec<u8> {
        // prevent screenshot hangs
        let screenshot_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(30),
            Page::take_screenshot(
                self,
                full_page,
                omit_background,
                format,
                quality,
                output_path,
                clip,
            ),
        )
        .await;
        match screenshot_result {
            Ok(sb) => sb,
            _ => Default::default(),
        }
    }

    #[cfg(all(feature = "chrome", not(feature = "decentralized")))]
    /// Get the chrome page used. The feature flag `chrome` is required.
    pub fn get_chrome_page(&self) -> Option<&chromiumoxide::Page> {
        self.chrome_page.as_ref()
    }

    #[cfg(all(feature = "chrome", feature = "decentralized"))]
    /// Get the chrome page used. The feature flag `chrome` is required.
    pub fn get_chrome_page(&self) -> Option<&chromiumoxide::Page> {
        None
    }

    #[cfg(all(not(feature = "decentralized"), feature = "chrome"))]
    /// Close the chrome page used. Useful when storing the page with subscription usage. The feature flag `chrome_store_page` is required.
    pub async fn close_page(&mut self) {
        if let Some(page) = self.chrome_page.as_mut() {
            let _ = page
                .send_command(chromiumoxide::cdp::browser_protocol::page::CloseParams::default())
                .await;
        }
    }

    #[cfg(all(feature = "decentralized", feature = "chrome"))]
    /// Close the chrome page used. Useful when storing the page for subscription usage. The feature flag `chrome_store_page` is required.
    pub async fn close_page(&mut self) {}

    /// Page request is empty. On chrome an empty page has bare html markup.
    /// When the `balance` feature is active, a page whose HTML has been
    /// spooled to disk is *not* considered empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        match self.html.as_deref() {
            None => {
                #[cfg(all(feature = "balance", not(feature = "decentralized")))]
                if self.html_spool_path.is_some() {
                    return false;
                }
                true
            }
            Some(html) => {
                let html = html.trim_ascii();
                html.is_empty() || html.eq(*EMPTY_HTML) || html.eq(*EMPTY_HTML_BASIC)
            }
        }
    }

    /// Get the byte length of the page's HTML content without any I/O.
    ///
    /// When HTML is in memory, returns `html.len()` directly.
    /// When HTML is spooled to disk (`balance` feature), returns the cached
    /// byte length captured at spool time.
    /// Returns `0` when the page is truly empty.
    #[inline]
    pub fn size(&self) -> usize {
        if let Some(ref html) = self.html {
            return html.len();
        }
        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        {
            self.content_byte_len
        }
        #[cfg(any(not(feature = "balance"), feature = "decentralized"))]
        {
            0
        }
    }

    /// Url getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url(&self) -> &str {
        &self.url
    }

    #[cfg(not(feature = "headers"))]
    /// Get the timeout required for rate limiting. The max duration is 30 seconds for delay respecting. Requires the feature flag `headers`.
    pub fn get_timeout(&self) -> Option<Duration> {
        if self.status_code == 429 {
            return Some(Duration::from_millis(2_500));
        } else if self.status_code == StatusCode::GATEWAY_TIMEOUT {
            return Some(Duration::from_millis(1_500));
        } else if self.status_code.as_u16() >= 598 {
            // Proxy/unknown connection errors (599, 598) - brief backoff before
            // same-proxy retry to avoid hammering a transiently broken tunnel.
            return Some(Duration::from_millis(500));
        }

        None
    }

    #[cfg(feature = "headers")]
    /// Get the timeout required for rate limiting. The max duration is 30 seconds for delay respecting. Requires the feature flag `headers`.
    pub fn get_timeout(&self) -> Option<Duration> {
        if self.status_code == 429 {
            const MAX_TIMEOUT: Duration = Duration::from_secs(30);
            if let Some(headers) = &self.headers {
                if let Some(retry_after) = headers.get(reqwest::header::RETRY_AFTER) {
                    if let Ok(retry_after_str) = retry_after.to_str() {
                        if let Ok(seconds) = retry_after_str.parse::<u64>() {
                            return Some(Duration::from_secs(seconds).min(MAX_TIMEOUT));
                        }
                        if let Ok(date) = httpdate::parse_http_date(retry_after_str) {
                            if let Ok(duration) = date.duration_since(std::time::SystemTime::now())
                            {
                                return Some(duration.min(MAX_TIMEOUT));
                            }
                        }
                    }
                }
            };
            // Fall back to the same default as the `headers`-disabled path
            // when the server didn't send a usable Retry-After. Without this,
            // enabling the `headers` feature silently disables 429 backoff.
            return Some(Duration::from_millis(2_500));
        } else if self.status_code == StatusCode::GATEWAY_TIMEOUT {
            return Some(Duration::from_millis(1_500));
        } else if self.status_code.as_u16() >= 598 {
            // Proxy/unknown connection errors (599, 598) - brief backoff before
            // same-proxy retry to avoid hammering a transiently broken tunnel.
            return Some(Duration::from_millis(500));
        }

        None
    }

    /// Url getter for page after redirects.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_final(&self) -> &str {
        match self.final_redirect_destination.as_ref() {
            Some(u) => u,
            _ => &self.url,
        }
    }

    /// Set the external domains to treat as one
    pub fn set_external(&mut self, external_domains_caseless: Arc<HashSet<CaseInsensitiveString>>) {
        self.external_domains_caseless = external_domains_caseless;
    }

    /// Set the html directly of the page
    pub fn set_html_bytes(&mut self, html: Option<Vec<u8>>) {
        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        {
            // Subtract old tracked bytes if this page owns them.
            if self.balance_bytes_tracked {
                if let Some(old) = &self.html {
                    crate::utils::html_spool::track_bytes_sub(old.len());
                }
                self.balance_bytes_tracked = false;
            }
            // Drop the spool guard (deletes the temp file automatically).
            self.html_spool_path = None;
        }
        self.html = html.map(bytes::Bytes::from);
        self.binary_file = self
            .html
            .as_deref()
            .is_some_and(crate::utils::is_binary_body);
        self.is_valid_utf8 = self
            .html
            .as_deref()
            .is_some_and(|b| simdutf8::basic::from_utf8(b).is_ok());
        self.is_xml = self
            .html
            .as_deref()
            .is_some_and(|b| b.starts_with(b"<?xml"));
        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        {
            // Refresh the cached length so `size()` stays accurate across
            // later spools without needing to peek the bytes again.
            self.content_byte_len = self.html.as_ref().map_or(0, |h| h.len());
            if let Some(ref h) = self.html {
                crate::utils::html_spool::track_bytes_add(h.len());
                self.balance_bytes_tracked = true;
            }
        }
    }

    /// Offload this page's HTML to a temporary file on disk and release the
    /// in-memory buffer.  Returns `true` if the spool succeeded.
    ///
    /// The spool file lives under `{SPIDER_HTML_SPOOL_DIR || /tmp}/spider_html_<pid>/`
    /// and is deleted as soon as the content is consumed (via
    /// [`ensure_html_loaded`](Self::ensure_html_loaded) or link extraction).
    /// Offload HTML to disk synchronously.  Used by tests and non-async
    /// consumers.  **Prefer [`spool_html_to_disk_async`]** in async paths.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub fn spool_html_to_disk(&mut self) -> bool {
        let html = match self.html.as_ref() {
            Some(h) if !h.is_empty() => h,
            _ => return false,
        };
        // Chrome returns a bare `<html><head></head><body></body></html>` shell
        // for pages that never produced content.  Spooling that to disk and
        // then claiming the page has content (via `html_spool_path.is_some()`)
        // is wrong — treat it as empty and skip the spool entirely.
        {
            let trimmed = html.trim_ascii();
            if trimmed.is_empty() || trimmed == *EMPTY_HTML || trimmed == *EMPTY_HTML_BASIC {
                return false;
            }
        }
        if self.html_spool_path.is_some() {
            return false;
        }
        let path = crate::utils::html_spool::next_spool_path();
        if crate::utils::html_spool::spool_write(&path, html).is_ok() {
            let len = html.len();
            self.content_byte_len = len;
            self.html = None;
            crate::utils::html_spool::track_bytes_sub(len);
            crate::utils::html_spool::track_page_spooled();
            self.html_spool_path = Some(HtmlSpoolGuard::new(
                path,
                crate::utils::html_spool::current_website_spool_dir(),
            ));
            // Bytes are now on disk, not in memory — clear the tracking
            // flag so Drop does not double-subtract.
            self.balance_bytes_tracked = false;
            true
        } else {
            false
        }
    }

    /// Offload HTML to disk using `tokio::fs` — fully async, never blocks
    /// a tokio worker thread.  Used by `channel_send_page` and all internal
    /// crawl paths.
    ///
    /// The write path uses
    /// [`spool_write_streaming_vitals`](crate::utils::html_spool::spool_write_streaming_vitals)
    /// which computes `byte_len`, `is_valid_utf8`, `binary_file`, and
    /// `is_xml` **inline with the disk write** — one single-pass scan
    /// interleaved with I/O instead of a separate linear UTF-8 validation
    /// over the whole buffer.  The vitals are mirrored onto the `Page`
    /// struct so subsequent disk-aware accessors never re-validate.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub async fn spool_html_to_disk_async(&mut self) -> bool {
        let html = match self.html.as_ref() {
            Some(h) if !h.is_empty() => h,
            _ => return false,
        };
        // Chrome returns a bare `<html><head></head><body></body></html>` shell
        // for pages that never produced content.  Spooling that to disk and
        // then claiming the page has content (via `html_spool_path.is_some()`)
        // is wrong — treat it as empty and skip the spool entirely.
        {
            let trimmed = html.trim_ascii();
            if trimmed.is_empty() || trimmed == *EMPTY_HTML || trimmed == *EMPTY_HTML_BASIC {
                return false;
            }
        }
        if self.html_spool_path.is_some() {
            return false;
        }
        let path = crate::utils::html_spool::next_spool_path();
        match crate::utils::html_spool::spool_write_streaming_vitals(&path, html.as_ref()).await {
            Ok(vitals) => {
                // Mirror the streaming-computed vitals onto the page.  They
                // match the values captured at build time for the same
                // bytes, so this is strictly a no-behavior-change refresh
                // that keeps future accessors from touching the disk.
                self.content_byte_len = vitals.byte_len;
                self.is_valid_utf8 = vitals.is_valid_utf8;
                self.binary_file = vitals.binary_file;
                self.is_xml = vitals.is_xml;
                self.html = None;
                crate::utils::html_spool::track_bytes_sub(vitals.byte_len);
                crate::utils::html_spool::track_page_spooled();
                self.html_spool_path = Some(HtmlSpoolGuard::new(
                    path,
                    crate::utils::html_spool::current_website_spool_dir(),
                ));
                // Bytes are now on disk, not in memory — clear the tracking
                // flag so Drop does not double-subtract.
                self.balance_bytes_tracked = false;
                true
            }
            Err(_) => false,
        }
    }

    /// Reload HTML from disk spool into memory (sync).  Returns `true` if
    /// content was reloaded (or was already in memory).  The spool file is
    /// deleted after a successful reload.
    ///
    /// **Prefer [`ensure_html_loaded_async`](Self::ensure_html_loaded_async)**
    /// in async crawl paths to avoid blocking the tokio runtime on disk I/O.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub fn ensure_html_loaded(&mut self) -> bool {
        if self.html.is_some() {
            return true;
        }
        if let Some(guard) = self.html_spool_path.as_ref() {
            if let Some(path) = guard.path() {
                match crate::utils::html_spool::spool_read_bytes(path) {
                    Ok(bytes) => {
                        crate::utils::html_spool::track_bytes_add(bytes.len());
                        self.html = Some(bytes);
                        self.html_spool_path = None;
                        self.balance_bytes_tracked = true;
                        true
                    }
                    Err(_) => {
                        self.html_spool_path = None;
                        false
                    }
                }
            } else {
                self.html_spool_path = None;
                false
            }
        } else {
            false
        }
    }

    /// Async variant of [`ensure_html_loaded`](Self::ensure_html_loaded).
    /// Routes through `uring_fs` for true kernel-async I/O on Linux;
    /// falls back to `tokio::fs` on other platforms.  Used internally
    /// by async crawl paths.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub async fn ensure_html_loaded_async(&mut self) -> bool {
        if self.html.is_some() {
            return true;
        }
        if let Some(guard) = self.html_spool_path.as_ref() {
            if let Some(path) = guard.path() {
                let path_buf = path.to_path_buf();
                match crate::utils::html_spool::spool_read_bytes_async(path_buf).await {
                    Ok(bytes) => {
                        crate::utils::html_spool::track_bytes_add(bytes.len());
                        self.html = Some(bytes);
                        self.html_spool_path = None;
                        self.balance_bytes_tracked = true;
                        true
                    }
                    Err(_) => {
                        self.html_spool_path = None;
                        false
                    }
                }
            } else {
                self.html_spool_path = None;
                false
            }
        } else {
            false
        }
    }

    /// Whether this page's HTML currently lives on disk rather than in memory.
    /// Always returns `false` when the `balance` feature is not enabled.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    #[inline]
    pub fn is_html_on_disk(&self) -> bool {
        self.html.is_none() && self.html_spool_path.is_some()
    }

    /// Whether this page's HTML currently lives on disk rather than in memory.
    /// Always returns `false` when the `balance` feature is not enabled or
    /// the `decentralized` feature is active.
    #[cfg(any(not(feature = "balance"), feature = "decentralized"))]
    #[inline]
    pub fn is_html_on_disk(&self) -> bool {
        false
    }

    /// Check if this page contains binary content, even when the HTML is
    /// spooled to disk.
    ///
    /// Zero disk I/O: `binary_file` is snapshotted at build/spool time
    /// (before bytes leave memory) so spooled pages just read the cached
    /// flag.  In-memory pages also trust the flag by default; they only
    /// fall back to a magic-number re-scan when the flag is unset AND
    /// bytes are available (covers pages whose `html` was assigned after
    /// construction without going through `set_html_bytes`).
    #[inline]
    pub fn is_binary_spool_aware(&self) -> bool {
        if self.binary_file {
            return true;
        }
        match self.html.as_deref() {
            Some(bytes) => crate::utils::is_binary_body(bytes),
            None => false,
        }
    }

    /// Return the path to the disk-spooled HTML file, if any.
    ///
    /// Useful for consumers that receive a `Page` via a broadcast channel
    /// and want to stream the HTML directly (e.g. feeding chunks to
    /// `lol_html::HtmlRewriter::write()`).
    ///
    /// The path is valid as long as this `Page` (or its clone) is alive.
    /// Once the page is dropped the spool file is automatically deleted.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    #[inline]
    pub fn get_html_spool_path(&self) -> Option<&std::path::Path> {
        self.html_spool_path.as_ref().and_then(|guard| guard.path())
    }

    /// Stream the HTML content in fixed-size chunks to a caller-supplied
    /// callback, regardless of whether the HTML lives in memory or on disk.
    ///
    /// This is the recommended way for channel subscribers to process large
    /// pages without loading the entire content into memory at once.
    ///
    /// The callback receives each chunk as `&[u8]` and returns `true` to
    /// continue or `false` to stop early.  Returns the total number of
    /// bytes fed to the callback, or `0` if the page has no content.
    ///
    /// ```rust,ignore
    /// // Example: streaming to lol_html
    /// let mut rewriter = lol_html::HtmlRewriter::new(settings, |_| {});
    /// page.stream_html_bytes(65536, |chunk| {
    ///     rewriter.write(chunk).is_ok()
    /// });
    /// let _ = rewriter.end();
    /// ```
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub fn stream_html_bytes<F>(&self, chunk_size: usize, mut cb: F) -> usize
    where
        F: FnMut(&[u8]) -> bool,
    {
        // Fast path: HTML is in memory — chunk it directly.
        if let Some(ref html) = self.html {
            let mut total = 0usize;
            for chunk in html.chunks(chunk_size.max(1)) {
                total = total.saturating_add(chunk.len());
                if !cb(chunk) {
                    break;
                }
            }
            return total;
        }

        // Disk path: stream from spool file.
        if let Some(ref guard) = self.html_spool_path {
            if let Some(path) = guard.path() {
                return crate::utils::html_spool::spool_stream_chunks(path, chunk_size, cb)
                    .unwrap_or(0);
            }
        }

        0
    }

    /// Stream the HTML content in fixed-size chunks to a caller-supplied
    /// callback.  Works the same as
    /// [`stream_html_bytes`](Self::stream_html_bytes) but is available
    /// without the `balance` feature — it simply chunks the in-memory HTML.
    #[cfg(any(not(feature = "balance"), feature = "decentralized"))]
    pub fn stream_html_bytes<F>(&self, chunk_size: usize, mut cb: F) -> usize
    where
        F: FnMut(&[u8]) -> bool,
    {
        if let Some(ref html) = self.html {
            let mut total = 0usize;
            for chunk in html.chunks(chunk_size.max(1)) {
                total = total.saturating_add(chunk.len());
                if !cb(chunk) {
                    break;
                }
            }
            return total;
        }
        0
    }

    /// Async version of [`stream_html_bytes`](Self::stream_html_bytes).
    /// Uses `tokio::fs` for the disk path — never blocks a tokio worker.
    ///
    /// The callback receives each chunk as `&[u8]` and returns `true` to
    /// continue or `false` to stop early.
    ///
    /// ```rust,ignore
    /// // Example: fully async streaming
    /// let mut rewriter = lol_html::HtmlRewriter::new(settings, |_| {});
    /// page.stream_html_bytes_async(65536, |chunk| {
    ///     rewriter.write(chunk).is_ok()
    /// }).await;
    /// let _ = rewriter.end();
    /// ```
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub async fn stream_html_bytes_async<F>(&self, chunk_size: usize, mut cb: F) -> usize
    where
        F: FnMut(&[u8]) -> bool,
    {
        // Fast path: HTML is in memory.
        if let Some(ref html) = self.html {
            let mut total = 0usize;
            for chunk in html.chunks(chunk_size.max(1)) {
                total = total.saturating_add(chunk.len());
                if !cb(chunk) {
                    break;
                }
            }
            return total;
        }

        // Disk path: unified read — io_uring fast path on Linux,
        // tokio::fs streaming fallback elsewhere.  Only entered when
        // the page's HTML is spooled to disk.
        if let Some(ref guard) = self.html_spool_path {
            if let Some(path) = guard.path() {
                let chunk_size = chunk_size.max(1);
                let mut total = 0usize;

                let _ = crate::utils::uring_fs::read_file_chunked(
                    path.display().to_string(),
                    chunk_size,
                    |chunk| {
                        total = total.saturating_add(chunk.len());
                        cb(chunk)
                    },
                )
                .await;

                return total;
            }
        }

        0
    }

    /// Async version of [`stream_html_bytes`](Self::stream_html_bytes).
    /// Without the `balance` feature this simply chunks the in-memory HTML
    /// (no disk path exists).
    #[cfg(any(not(feature = "balance"), feature = "decentralized"))]
    pub async fn stream_html_bytes_async<F>(&self, chunk_size: usize, cb: F) -> usize
    where
        F: FnMut(&[u8]) -> bool,
    {
        self.stream_html_bytes(chunk_size, cb)
    }

    /// Async version of [`get_html`](Self::get_html).  When HTML is on disk,
    /// reads via `tokio::fs` instead of blocking.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub async fn get_html_async(&self) -> String {
        if let Some(bytes) = self.html.as_deref() {
            return if self.is_valid_utf8 {
                // Safety: UTF-8 validated once at construction time.
                unsafe { std::str::from_utf8_unchecked(bytes) }.to_string()
            } else {
                auto_encoder::auto_encode_bytes(bytes)
            };
        }
        if let Some(guard) = &self.html_spool_path {
            if let Some(path) = guard.path() {
                if let Ok(bytes) =
                    crate::utils::html_spool::spool_read_async(path.to_path_buf()).await
                {
                    return if self.is_valid_utf8 {
                        // Disk-spooled bytes preserve the encoding from
                        // the original response, so the cached flag is
                        // still valid.
                        unsafe { String::from_utf8_unchecked(bytes) }
                    } else {
                        String::from_utf8(bytes)
                            .unwrap_or_else(|e| auto_encoder::auto_encode_bytes(&e.into_bytes()))
                    };
                }
            }
        }
        String::new()
    }

    /// Async version of [`get_html`](Self::get_html).  Without balance this
    /// delegates to the sync version (no disk path).
    #[cfg(any(not(feature = "balance"), feature = "decentralized"))]
    pub async fn get_html_async(&self) -> String {
        self.get_html()
    }

    /// Set the url directly of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url(&mut self, url: String) {
        self.url = url;
    }

    /// Set the url directly parsed url of the page.
    /// Uses the final redirect destination when available, so that relative link
    /// resolution and parent_host_match use the actual origin after redirects.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url_parsed_direct(&mut self) {
        let effective_url = match &self.final_redirect_destination {
            Some(u) => u.as_str(),
            None => &self.url,
        };
        if let Ok(base) = Url::parse(effective_url) {
            self.base = Some(base);
        }
    }

    /// Set the url directly parsed url of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url_parsed_direct_empty(&mut self) {
        if !self.base.is_some() && !self.url.is_empty() {
            self.set_url_parsed_direct()
        }
    }

    /// Set the url directly parsed url of the page.
    #[cfg(feature = "decentralized")]
    pub fn set_url_parsed_direct(&mut self) {}

    /// Set the url directly parsed url of the page. Useful for transforming the content and rewriting the url.
    #[cfg(feature = "decentralized")]
    pub fn set_url_parsed_direct_empty(&mut self) {}

    /// Set the url directly parsed url of the page. Useful for transforming the content and rewriting the url.
    #[cfg(not(feature = "decentralized"))]
    pub fn set_url_parsed(&mut self, url_parsed: Url) {
        self.base = Some(url_parsed);
    }

    /// Parsed URL getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_parsed_ref(&self) -> &Option<Url> {
        &self.base
    }

    /// Parsed URL getter for page.
    #[cfg(not(feature = "decentralized"))]
    pub fn get_url_parsed(&mut self) -> &Option<Url> {
        if self.base.is_none() && !self.url.is_empty() {
            self.base = Url::parse(&self.url).ok();
        }
        &self.base
    }

    /// Parsed URL getter for page.
    #[cfg(feature = "decentralized")]
    pub fn get_url_parsed(&self) -> &Option<Url> {
        &None
    }

    /// Parsed URL getter for page.
    #[cfg(feature = "decentralized")]
    pub fn get_url_parsed_ref(&self) -> &Option<Url> {
        &None
    }

    /// Take the parsed url.
    #[cfg(not(feature = "decentralized"))]
    pub fn take_url(&mut self) -> Option<Url> {
        self.base.take()
    }

    /// Take the parsed url.
    #[cfg(feature = "decentralized")]
    pub fn take_url(&mut self) -> Option<Url> {
        None
    }

    #[cfg(feature = "decentralized")]
    /// URL getter for page.
    pub fn get_url(&self) -> &str {
        &self.url
    }

    /// Html getter for bytes on the page.
    ///
    /// Returns `None` when HTML is spooled to disk.  Use [`get_html`],
    /// [`get_html_async`], or [`stream_html_bytes`] for disk-aware access.
    pub fn get_bytes(&self) -> Option<&[u8]> {
        self.html.as_deref()
    }

    /// Html getter for bytes on the page as string.
    ///
    /// When the `balance` feature is active and the HTML was spooled to disk,
    /// this transparently reads from the temporary file and returns the
    /// content.  The spool file is **not** deleted here (use
    /// [`ensure_html_loaded`](Self::ensure_html_loaded) to reload + delete).
    pub fn get_html(&self) -> String {
        if let Some(bytes) = self.html.as_deref() {
            return if self.is_valid_utf8 {
                unsafe { std::str::from_utf8_unchecked(bytes) }.to_string()
            } else {
                auto_encoder::auto_encode_bytes(bytes)
            };
        }
        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        if let Some(guard) = &self.html_spool_path {
            if let Some(path) = guard.path() {
                if let Ok(bytes) = crate::utils::html_spool::spool_read(path) {
                    return if self.is_valid_utf8 {
                        unsafe { String::from_utf8_unchecked(bytes) }
                    } else {
                        String::from_utf8(bytes)
                            .unwrap_or_else(|e| auto_encoder::auto_encode_bytes(&e.into_bytes()))
                    };
                }
            }
        }
        String::new()
    }

    /// Content getter — returns the page body as a string.
    ///
    /// This is an alias for [`get_html`](Self::get_html) that works with any
    /// return format (HTML, markdown, text, etc.) set via
    /// [`SpiderCloudConfig::with_return_format`](crate::configuration::SpiderCloudConfig::with_return_format)
    /// or transformed locally with `spider_transformations`.
    #[inline]
    pub fn get_content(&self) -> String {
        self.get_html()
    }

    /// Html getter that avoids allocation when the content is already valid UTF-8.
    /// Returns `Cow::Borrowed` for UTF-8 content (common case), `Cow::Owned` when
    /// encoding conversion is needed or content is loaded from a disk spool.
    pub fn get_html_cow(&self) -> std::borrow::Cow<'_, str> {
        match self.html.as_deref() {
            Some(bytes) => {
                if self.is_valid_utf8 {
                    std::borrow::Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(bytes) })
                } else {
                    std::borrow::Cow::Owned(auto_encoder::auto_encode_bytes(bytes))
                }
            }
            None => {
                #[cfg(all(feature = "balance", not(feature = "decentralized")))]
                if let Some(guard) = &self.html_spool_path {
                    if let Some(path) = guard.path() {
                        if let Ok(bytes) = crate::utils::html_spool::spool_read(path) {
                            return std::borrow::Cow::Owned(if self.is_valid_utf8 {
                                unsafe { String::from_utf8_unchecked(bytes) }
                            } else {
                                String::from_utf8(bytes).unwrap_or_else(|e| {
                                    auto_encoder::auto_encode_bytes(&e.into_bytes())
                                })
                            });
                        }
                    }
                }
                std::borrow::Cow::Borrowed("")
            }
        }
    }

    /// Html getter for page to u8.
    ///
    /// **Disk-spool caveat (`balance` feature):** this accessor only returns
    /// the in-memory buffer.  Once HTML has been spooled to a temporary file
    /// via the balance feature, `self.html` is `None` and this method returns
    /// an empty slice — loading the file back would defeat the point of
    /// spooling.  Disk-aware callers should use
    /// [`stream_html_bytes_async`](Self::stream_html_bytes_async),
    /// [`get_html_async`](Self::get_html_async), or
    /// [`get_content`](Self::get_content) instead.  Pre-computed vitals
    /// (`binary_file`, `is_valid_utf8`, `is_xml`, `size()`) survive the
    /// spool and do not require disk I/O.
    pub fn get_html_bytes_u8(&self) -> &[u8] {
        match self.html.as_deref() {
            Some(html) => html,
            _ => Default::default(),
        }
    }

    /// Content getter as raw bytes — alias for [`get_html_bytes_u8`](Self::get_html_bytes_u8).
    ///
    /// Works with any return format (HTML, markdown, text, etc.).
    #[inline]
    pub fn get_content_bytes(&self) -> &[u8] {
        self.get_html_bytes_u8()
    }

    /// Get content for a specific return format from a multi-format response.
    ///
    /// Returns `None` if multi-format was not requested or the format is not present.
    /// Use [`with_return_formats`](crate::configuration::SpiderCloudConfig::with_return_formats)
    /// on `SpiderCloudConfig` to request multiple formats.
    #[cfg(feature = "spider_cloud")]
    #[inline]
    pub fn get_content_for(&self, format: &str) -> Option<String> {
        self.content_map.as_ref().and_then(|map| {
            map.get(format).map(|b| {
                simdutf8::basic::from_utf8(b)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| auto_encoder::auto_encode_bytes(b))
            })
        })
    }

    /// Get content for a specific return format as raw bytes.
    ///
    /// Returns `None` if multi-format was not requested or the format is not present.
    #[cfg(feature = "spider_cloud")]
    #[inline]
    pub fn get_content_bytes_for(&self, format: &str) -> Option<&[u8]> {
        self.content_map
            .as_ref()
            .and_then(|map| map.get(format).map(|b| b.as_ref()))
    }

    /// Check if this page has multi-format content available.
    #[cfg(feature = "spider_cloud")]
    #[inline]
    pub fn has_content_map(&self) -> bool {
        self.content_map.as_ref().is_some_and(|m| !m.is_empty())
    }

    /// Compute an HTML quality score (0–100) for this page.
    ///
    /// Uses status code, content length, structural HTML checks,
    /// and anti-bot detection to score the response.
    #[cfg(feature = "parallel_backends")]
    #[inline]
    pub fn quality_score(&self) -> u16 {
        crate::utils::parallel_backends::html_quality_score(
            self.html.as_deref(),
            self.status_code,
            &self.anti_bot_tech,
            self.get_url(),
        )
    }

    /// Modify xml - html.
    #[cfg(all(
        feature = "sitemap",
        feature = "chrome",
        not(feature = "decentralized")
    ))]
    pub(crate) fn modify_xml_html(&mut self) -> &[u8] {
        if let Some(html_bytes) = self.html.take() {
            const XML_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

            let xml = html_bytes.as_ref();
            if let Ok(xml_str) = simdutf8::basic::from_utf8(xml) {
                let stripped = xml_str
                    .strip_prefix(XML_DECL)
                    .map(|f| f.trim_start())
                    .unwrap_or(xml_str);
                // Zero-copy: Bytes::slice shares the same Arc backing buffer.
                let offset = stripped.as_ptr() as usize - xml.as_ptr() as usize;
                self.html = Some(html_bytes.slice(offset..offset + stripped.len()));
            } else {
                self.html = Some(html_bytes);
            }
        }

        self.html.as_deref().unwrap_or_default()
    }

    /// Get the response events mapped.
    #[cfg(feature = "chrome")]
    pub fn get_responses(&self) -> &Option<hashbrown::HashMap<String, f64>> {
        &self.response_map
    }

    /// Get the metadata found on the page.
    pub fn get_metadata(&self) -> &Option<Box<Metadata>> {
        &self.metadata
    }

    /// Get the response events mapped.
    #[cfg(feature = "chrome")]
    pub fn get_request(&self) -> &Option<hashbrown::HashMap<String, f64>> {
        &self.request_map
    }

    /// Html getter for getting the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS. This fallsback to get_html without the `encoding` flag enabled.
    #[cfg(feature = "encoding")]
    pub fn get_html_encoded(&self, label: &str) -> String {
        get_html_encoded(&self.html, label)
    }

    /// Html getter for getting the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS. This fallsback to get_html without the `encoding` flag enabled.
    #[cfg(not(feature = "encoding"))]
    pub fn get_html_encoded(&self, _label: &str) -> String {
        self.get_html()
    }

    /// Set the elasped duration of the page since scraped from duration.
    #[inline]
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn set_duration_elapsed(&mut self, scraped_at: Option<Instant>) {
        self.duration = scraped_at;
    }

    /// Set the elasped duration of the page since scraped from duration.
    #[inline]
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn set_duration_elapsed_from_duration(&mut self, elapsed: Option<std::time::Duration>) {
        self.duration = elapsed.map(|d| Instant::now().checked_sub(d).unwrap_or_else(Instant::now));
    }

    /// Get the elasped duration of the page since scraped.
    #[cfg(all(feature = "time", not(feature = "decentralized")))]
    pub fn get_duration_elapsed(&self) -> Duration {
        self.duration
            .as_ref()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    /// Set the elapsed duration of the page since scraped from duration.
    #[inline]
    #[cfg(all(feature = "time", feature = "decentralized"))]
    pub fn set_duration_elapsed(&mut self, scraped_at: Option<Instant>) {
        self.duration = scraped_at;
    }

    /// Set the elapsed duration of the page since scraped from duration.
    #[inline]
    #[cfg(all(feature = "time", feature = "decentralized"))]
    pub fn set_duration_elapsed_from_duration(&mut self, elapsed: Option<std::time::Duration>) {
        self.duration = elapsed.map(|d| Instant::now().checked_sub(d).unwrap_or_else(Instant::now));
    }

    /// Get the elapsed duration of the page since scraped.
    #[cfg(all(feature = "time", feature = "decentralized"))]
    pub fn get_duration_elapsed(&self) -> Duration {
        self.duration
            .as_ref()
            .map(|t| t.elapsed())
            .unwrap_or_default()
    }

    /// Find the links as a stream using string resource validation for XML files.
    ///
    /// Thin `&[u8]` wrapper around the generic
    /// [`links_stream_xml_from_reader`](Self::links_stream_xml_from_reader)
    /// so existing in-memory callers keep their shape.  Disk-spooled pages
    /// should call
    /// [`links_stream_xml_from_disk`](Self::links_stream_xml_from_disk)
    /// instead to avoid loading the full file into memory.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_xml_links_stream_base<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        xml: &[u8],
        map: &mut HashSet<A>,
        base: &Option<Box<Url>>,
    ) {
        self.links_stream_xml_from_reader(selectors, xml, map, base)
            .await;
    }

    /// Disk-streaming XML link extractor.  Opens the spool file as a buffered
    /// async reader and feeds chunks into `quick_xml` via `read_event_into_async`
    /// — never materialises the full XML in memory.  Used by disk-spooled
    /// sitemaps / feeds when the `balance` feature offloads large responses.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub async fn links_stream_xml_from_disk<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        spool_path: std::path::PathBuf,
        map: &mut HashSet<A>,
        base: &Option<Box<Url>>,
    ) {
        match tokio::fs::File::open(&spool_path).await {
            Ok(file) => {
                let reader = tokio::io::BufReader::with_capacity(*STREAMING_CHUNK_SIZE, file);
                self.links_stream_xml_from_reader(selectors, reader, map, base)
                    .await;
            }
            Err(_) => {
                // Missing or unreadable spool file — caller treats an empty
                // map as "no links", matching the behaviour of the previous
                // full-read code path when `uring_fs::read_file` errored.
            }
        }
    }

    /// Internal generic XML extractor.  Accepts any `AsyncBufRead` source,
    /// so in-memory `&[u8]` callers and disk-streaming `BufReader<File>`
    /// callers share one implementation.
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_xml_from_reader<
        R: tokio::io::AsyncBufRead + Unpin + Send,
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        source: R,
        map: &mut HashSet<A>,
        base: &Option<Box<Url>>,
    ) {
        use quick_xml::events::Event;
        use quick_xml::reader::NsReader;

        let mut reader = NsReader::from_reader(source);

        reader.config_mut().trim_text(true);

        // Take the thread-local buffer (owned, safe across awaits).
        // If re-entered or migrated, we just get an empty Vec — no panic.
        let mut buf = XML_PARSE_BUF.with(|c| c.take());

        let parent_host = &selectors.1[0];
        let parent_host_scheme = &selectors.1[1];
        let base_input_domain = &selectors.2;
        let sub_matcher = &selectors.0;

        let mut is_link_tag = false;
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let base = if base.is_some() {
            base.as_deref()
        } else {
            self.set_url_parsed_direct_empty();
            let base = self.get_url_parsed_ref().as_ref();
            base
        };

        loop {
            match reader.read_event_into_async(&mut buf).await {
                Ok(e) => match e {
                    Event::Start(e) => {
                        let (_, local) = reader.resolver().resolve_element(e.name());

                        if local.as_ref() == b"link" {
                            is_link_tag = true;
                        }
                    }
                    Event::Text(e) => {
                        if is_link_tag {
                            if let Ok(v) = e.decode() {
                                push_link_verify(
                                    &base,
                                    &v,
                                    map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    &self.external_domains_caseless,
                                    false,
                                    &mut links_pages,
                                    true,
                                );
                            }
                        }
                    }
                    Event::End(ref e) => {
                        let (_, local) = reader.resolver().resolve_element(e.name());

                        if local.as_ref() == b"link" {
                            is_link_tag = false;
                        }
                    }
                    Event::Eof => {
                        break;
                    }
                    _ => (),
                },
                _ => break,
            }
            buf.clear();
        }

        // Return the buffer to the thread-local pool (retains capacity for reuse).
        buf.clear();
        XML_PARSE_BUF.with(|c| c.set(buf));

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
        }
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    pub async fn links_stream_base<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        html: &[u8],
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !html.is_empty() {
            if self.is_xml {
                self.links_stream_xml_links_stream_base(selectors, html, &mut map, base)
                    .await;
            } else {
                let base_input_url = tokio::sync::OnceCell::new();
                let base = base.as_deref();
                let xml_file = self.get_url().ends_with(".xml");

                // Snapshot self-borrows up front so the rewriter
                // closures (which capture &self.external_domains_caseless
                // + the parsed URL) hold one continuous immutable
                // borrow on `self`, dropped when the rewriter ends.
                self.set_url_parsed_direct_empty();
                let original_page = self.get_url_parsed_ref().as_ref();
                let external_domains_caseless = &self.external_domains_caseless;

                let element_content_handlers = build_link_extract_handlers(
                    LinkExtractCtx {
                        selectors,
                        external_domains_caseless,
                        map: &mut map,
                        links_pages: &mut links_pages,
                        base_input_url: &base_input_url,
                        base,
                        original_page,
                        ssg_raw_src_cell: None,
                        ssg_resolved_path_cell: None,
                        xml_file,
                        full_resources: false,
                        skip_links: false,
                    },
                    &mut meta_title,
                    &mut meta_description,
                    &mut meta_og_image,
                );

                let rewriter_settings = lol_html::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut wrote_error = false;

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings, |_c: &[u8]| {});

                let should_yield = html.len() > REWRITER_YIELD_THRESHOLD;

                for (i, chunk) in html.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
                    if rewriter.write(chunk).is_err() {
                        wrote_error = true;
                        break;
                    }
                    if should_yield && i % REWRITER_YIELD_INTERVAL == REWRITER_YIELD_INTERVAL - 1 {
                        tokio::task::yield_now().await;
                    }
                }

                if !wrote_error {
                    let _ = rewriter.end();
                }
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        map
    }

    /// Stream link extraction from a disk-spooled HTML file without loading
    /// the entire file into memory.  Uses the same lol_html rewriter setup
    /// as [`links_stream_base`] but feeds it buffered disk reads.
    ///
    /// The spool file is **not** deleted here — the caller or Drop handles
    /// cleanup so the same spool can serve multiple consumers if needed.
    #[cfg(all(not(feature = "decentralized"), feature = "balance"))]
    pub async fn links_stream_base_from_disk<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        spool_path: std::path::PathBuf,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        // XML path: stream from disk via BufReader<File> → quick_xml async
        // reader.  Never materialises the full document in memory.
        if self.is_xml {
            self.links_stream_xml_from_disk(selectors, spool_path.clone(), &mut map, base)
                .await;
        } else {
            let base_input_url = tokio::sync::OnceCell::new();
            let base = base.as_deref();
            let xml_file = self.get_url().ends_with(".xml");

            self.set_url_parsed_direct_empty();
            let original_page = self.get_url_parsed_ref().as_ref();
            let external_domains_caseless = &self.external_domains_caseless;

            let element_content_handlers = build_link_extract_handlers(
                LinkExtractCtx {
                    selectors,
                    external_domains_caseless,
                    map: &mut map,
                    links_pages: &mut links_pages,
                    base_input_url: &base_input_url,
                    base,
                    original_page,
                    ssg_raw_src_cell: None,
                    ssg_resolved_path_cell: None,
                    xml_file,
                    full_resources: false,
                    skip_links: false,
                },
                &mut meta_title,
                &mut meta_description,
                &mut meta_og_image,
            );

            let rewriter_settings = lol_html::Settings {
                element_content_handlers,
                adjust_charset_on_meta_tag: true,
                ..lol_html::send::Settings::new_for_handler_types()
            };

            let mut rewriter = lol_html::send::HtmlRewriter::new(rewriter_settings, |_c: &[u8]| {});

            let chunk_size = *STREAMING_CHUNK_SIZE;
            let mut wrote_error = false;
            let mut chunk_idx = 0usize;

            // Unified disk read: io_uring fast path on Linux (single
            // kernel-async read), tokio::fs streaming fallback elsewhere.
            let _ = crate::utils::uring_fs::read_file_chunked(
                spool_path.display().to_string(),
                chunk_size,
                |chunk| {
                    if rewriter.write(chunk).is_err() {
                        wrote_error = true;
                        return false;
                    }
                    chunk_idx += 1;
                    true
                },
            )
            .await;

            if !wrote_error {
                let _ = rewriter.end();
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        map
    }

    /// Disk-streaming variant of [`links_stream_base_ssg`] for spooled pages.
    /// Streams from the spool file in chunks via `lol_html` with SSG manifest
    /// detection — never loads the full page HTML into memory.
    #[cfg(all(feature = "balance", not(feature = "decentralized")))]
    pub async fn links_stream_base_from_disk_ssg<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        spool_path: std::path::PathBuf,
        client: &Client,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut map_ssg: HashSet<A> = HashSet::new();
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        // XML path: stream from disk via BufReader<File> → quick_xml async
        // reader — no full in-memory buffer.
        if self.is_xml {
            self.links_stream_xml_from_disk(selectors, spool_path.clone(), &mut map, base)
                .await;
        } else {
            let cell = tokio::sync::OnceCell::new();
            let base_input_url = tokio::sync::OnceCell::new();

            let parent_host = &selectors.1[0];
            let parent_host_scheme = &selectors.1[1];
            let base_input_domain = &selectors.2;
            let sub_matcher = &selectors.0;

            let base = base.as_deref();
            let xml_file = self.get_url().ends_with(".xml");

            self.set_url_parsed_direct_empty();
            let original_page = self.get_url_parsed_ref().as_ref();

            {
                // Inner scope so the helper's `&self.external_domains_caseless`
                // borrow ends before the post-rewriter SSG block re-borrows
                // self for `Page::new_page` and `push_link` into map_ssg.
                let external_domains_caseless = &self.external_domains_caseless;
                let element_content_handlers = build_link_extract_handlers(
                    LinkExtractCtx {
                        selectors,
                        external_domains_caseless,
                        map: &mut map,
                        links_pages: &mut links_pages,
                        base_input_url: &base_input_url,
                        base,
                        original_page,
                        ssg_raw_src_cell: None,
                        ssg_resolved_path_cell: Some(&cell),
                        xml_file,
                        full_resources: false,
                        skip_links: false,
                    },
                    &mut meta_title,
                    &mut meta_description,
                    &mut meta_og_image,
                );

                let rewriter_settings = lol_html::Settings {
                    element_content_handlers,
                    adjust_charset_on_meta_tag: true,
                    ..lol_html::send::Settings::new_for_handler_types()
                };

                let mut rewriter =
                    lol_html::send::HtmlRewriter::new(rewriter_settings, |_c: &[u8]| {});

                let chunk_size = *STREAMING_CHUNK_SIZE;
                let mut wrote_error = false;

                let _ = crate::utils::uring_fs::read_file_chunked(
                    spool_path.display().to_string(),
                    chunk_size,
                    |chunk| {
                        if rewriter.write(chunk).is_err() {
                            wrote_error = true;
                            return false;
                        }
                        true
                    },
                )
                .await;

                if !wrote_error {
                    let _ = rewriter.end();
                }
            }

            // Process SSG manifest if detected during streaming.
            if let Some(build_ssg_path) = cell.get() {
                if !build_ssg_path.is_empty() {
                    let build_page = Page::new_page(build_ssg_path, client).await;

                    for cap in SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8()) {
                        if let Some(matched) = cap.get(1) {
                            let href =
                                auto_encode_bytes(matched.as_bytes()).replace(r#"\u002F"#, "/");

                            let last_segment = crate::utils::get_last_segment(&href);

                            if !(last_segment.starts_with("[") && last_segment.ends_with("]")) {
                                let base = if relative_directory_url(&href) || base.is_none() {
                                    original_page
                                } else {
                                    base
                                };
                                let base = if base_input_url.initialized() {
                                    base_input_url.get()
                                } else {
                                    base
                                };

                                push_link(
                                    &base,
                                    &href,
                                    &mut map_ssg,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    &self.external_domains_caseless,
                                    &mut None,
                                );
                            }
                        }
                    }
                }
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
        }

        let valid_meta = meta_title.is_some()
            || meta_description.is_some()
            || meta_og_image.is_some()
            || self.get_metadata().is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() && self.metadata.is_some() {
                set_metadata(&mut self.metadata, &mut metadata_inner);
            }

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        map.extend(map_ssg);

        update_link_capacity_hint(map.len());

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_base_ssg<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        html: &[u8],
        client: &Client,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut map_ssg: HashSet<A> = HashSet::new();
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if !html.is_empty() {
            if self.is_xml {
                self.links_stream_xml_links_stream_base(selectors, html, &mut map, base)
                    .await;
            } else {
                let cell = tokio::sync::OnceCell::new();
                let base_input_url = tokio::sync::OnceCell::new();

                // the original url
                let parent_host = &selectors.1[0];
                // the host schemes
                let parent_host_scheme = &selectors.1[1];
                let base_input_domain = &selectors.2; // the domain after redirects
                let sub_matcher = &selectors.0;

                let base = base.as_deref();

                // original domain to match local pages.
                let original_page = {
                    self.set_url_parsed_direct_empty();
                    self.get_url_parsed_ref().as_ref()
                };

                let xml_file = self.get_url().ends_with(".xml");

                {
                    // Inner scope so the helper's borrows on `map`,
                    // `links_pages`, and `&self.external_domains_caseless`
                    // are released before the post-rewriter SSG block
                    // mutates `map_ssg` and re-reads them.
                    let external_domains_caseless = &self.external_domains_caseless;
                    let element_content_handlers = build_link_extract_handlers(
                        LinkExtractCtx {
                            selectors,
                            external_domains_caseless,
                            map: &mut map,
                            links_pages: &mut links_pages,
                            base_input_url: &base_input_url,
                            base,
                            original_page,
                            ssg_raw_src_cell: None,
                            ssg_resolved_path_cell: Some(&cell),
                            xml_file,
                            full_resources: false,
                            skip_links: false,
                        },
                        &mut meta_title,
                        &mut meta_description,
                        &mut meta_og_image,
                    );

                    let rewriter_settings = lol_html::Settings {
                        element_content_handlers,
                        adjust_charset_on_meta_tag: true,
                        ..lol_html::send::Settings::new_for_handler_types()
                    };

                    let mut rewriter =
                        lol_html::send::HtmlRewriter::new(rewriter_settings, |_c: &[u8]| {});

                    let mut wrote_error = false;
                    let should_yield = html.len() > REWRITER_YIELD_THRESHOLD;

                    for (i, chunk) in html.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
                        if rewriter.write(chunk).is_err() {
                            wrote_error = true;
                            break;
                        }
                        if should_yield
                            && i % REWRITER_YIELD_INTERVAL == REWRITER_YIELD_INTERVAL - 1
                        {
                            tokio::task::yield_now().await;
                        }
                    }

                    if !wrote_error {
                        let _ = rewriter.end();
                    }
                }

                if let Some(build_ssg_path) = cell.get() {
                    if !build_ssg_path.is_empty() {
                        let build_page = Page::new_page(build_ssg_path, client).await;

                        for cap in SSG_CAPTURE.captures_iter(build_page.get_html_bytes_u8()) {
                            if let Some(matched) = cap.get(1) {
                                let href =
                                    auto_encode_bytes(matched.as_bytes()).replace(r#"\u002F"#, "/");

                                let last_segment = crate::utils::get_last_segment(&href);

                                // we can pass in a static map of the dynamic SSG routes pre-hand, custom API endpoint to seed, or etc later.
                                if !(last_segment.starts_with("[") && last_segment.ends_with("]")) {
                                    let base = if relative_directory_url(&href) || base.is_none() {
                                        original_page
                                    } else {
                                        base
                                    };
                                    let base = if base_input_url.initialized() {
                                        base_input_url.get()
                                    } else {
                                        base
                                    };

                                    push_link(
                                        &base,
                                        &href,
                                        &mut map_ssg,
                                        &selectors.0,
                                        parent_host,
                                        parent_host_scheme,
                                        base_input_domain,
                                        sub_matcher,
                                        &self.external_domains_caseless,
                                        &mut None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
        }

        let valid_meta = meta_title.is_some()
            || meta_description.is_some()
            || meta_og_image.is_some()
            || self.get_metadata().is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() && self.metadata.is_some() {
                set_metadata(&mut self.metadata, &mut metadata_inner);
            }

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        map.extend(map_ssg);

        update_link_capacity_hint(map.len());

        map
    }

    /// Find the links as a stream using string resource validation and parsing the script for nextjs initial SSG paths.
    #[cfg(not(feature = "decentralized"))]
    pub async fn links_stream_ssg<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        client: &Client,
        prior_domain: &Option<Box<Url>>,
    ) -> HashSet<A> {
        if self.is_binary_spool_aware() {
            Default::default()
        } else if let Some(html_bytes) = self.html.take() {
            let result = self
                .links_stream_base_ssg(selectors, &html_bytes, client, prior_domain)
                .await;
            self.html = Some(html_bytes);
            result
        } else {
            // When HTML is on disk, stream the spool file in chunks through
            // lol_html.  The dedicated `_from_disk_ssg` variant avoids
            // allocating a full in-memory buffer — keeps the balance feature
            // honest: bytes stay on disk during link extraction.
            #[cfg(all(feature = "balance", not(feature = "decentralized")))]
            if let Some(ref guard) = self.html_spool_path {
                if let Some(path) = guard.path() {
                    return self
                        .links_stream_base_from_disk_ssg(
                            selectors,
                            path.to_path_buf(),
                            client,
                            prior_domain,
                        )
                        .await;
                }
            }
            Default::default()
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    pub async fn links_ssg(
        &mut self,
        selectors: &RelativeSelectors,
        client: &Client,
        prior_domain: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        let has_html = self.html.is_some();

        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        let has_html = has_html || self.html_spool_path.is_some();

        match has_html {
            false => Default::default(),
            true => {
                // When HTML is on disk, stream link extraction with SSG
                // support directly without loading full content into memory.
                #[cfg(all(feature = "balance", not(feature = "decentralized")))]
                if self.html.is_none() && self.html_spool_path.is_some() {
                    if let Some(ref guard) = self.html_spool_path {
                        if let Some(path) = guard.path() {
                            return self
                                .links_stream_base_from_disk_ssg(
                                    selectors,
                                    path.to_path_buf(),
                                    client,
                                    prior_domain,
                                )
                                .await;
                        }
                    }
                    return Default::default();
                }
                self.links_stream_ssg::<CaseInsensitiveString>(selectors, client, prior_domain)
                    .await
            }
        }
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), not(feature = "full_resources")))]
    pub async fn links_stream<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        if self.is_binary_spool_aware() {
            Default::default()
        } else if let Some(html_bytes) = self.html.take() {
            let result = self.links_stream_base(selectors, &html_bytes, base).await;
            self.html = Some(html_bytes);
            result
        } else {
            // When HTML is on disk, stream from the spool file without
            // loading the entire contents into memory.
            #[cfg(all(feature = "balance", not(feature = "decentralized")))]
            if let Some(ref guard) = self.html_spool_path {
                if let Some(path) = guard.path() {
                    return self
                        .links_stream_base_from_disk(selectors, path.to_path_buf(), base)
                        .await;
                }
            }
            Default::default()
        }
    }

    /// Find the links as a stream using string resource validation
    #[cfg(all(
        not(feature = "decentralized"),
        not(feature = "full_resources"),
        feature = "smart"
    ))]
    #[inline(always)]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub(crate) async fn links_stream_smart<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        configuration: &crate::configuration::Configuration,
        base: &Option<Box<Url>>,
        browser: &crate::features::chrome::OnceBrowser,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    ) -> (HashSet<A>, Option<f64>) {
        use lol_html::{element, text};
        use std::sync::atomic::Ordering;

        let mut bytes_transferred: Option<f64> = None;
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut inner_map: HashSet<A> = HashSet::new();
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        // Handle XML streaming first — can stream from memory or disk without
        // loading full bytes, then skip the HTML rewriter path entirely.
        if self.is_xml {
            if let Some(html_bytes_taken) = self.html.take() {
                self.links_stream_xml_links_stream_base(
                    selectors,
                    html_bytes_taken.as_ref(),
                    &mut map,
                    &base,
                )
                .await;
                self.html = Some(html_bytes_taken);
            } else {
                #[cfg(all(feature = "balance", not(feature = "decentralized")))]
                if let Some(ref guard) = self.html_spool_path {
                    if let Some(path) = guard.path() {
                        self.links_stream_xml_from_disk(
                            selectors,
                            path.to_path_buf(),
                            &mut map,
                            &base,
                        )
                        .await;
                    }
                }
            }
        } else {
            // When HTML is on disk (spooled), stream from spool file.
            // Chrome upgrade heuristic is skipped — spooled pages already
            // have rendered content.
            #[cfg(all(feature = "balance", not(feature = "decentralized")))]
            if self.html.is_none() && self.html_spool_path.is_some() {
                if let Some(ref guard) = self.html_spool_path {
                    if let Some(path) = guard.path() {
                        let disk_links: HashSet<A> = self
                            .links_stream_base_from_disk(selectors, path.to_path_buf(), base)
                            .await;
                        map.extend(disk_links);
                    }
                }
            }

            if let Some(html_bytes_taken) = self.html.take() {
                {
                    let base_input_url = tokio::sync::OnceCell::new();

                    let base_input_domain = &selectors.2;
                    let parent_frags = &selectors.1; // todo: allow mix match tpt
                    let parent_host = &parent_frags[0];
                    let parent_host_scheme = &parent_frags[1];
                    let sub_matcher = &selectors.0;

                    let base1 = base.as_deref();

                    // original domain to match local pages.
                    let original_page = {
                        self.set_url_parsed_direct_empty();
                        self.get_url_parsed_ref().as_ref().cloned()
                    };

                    // Borrow the shared Arc rather than cloning — the borrow
                    // is released when the rewriter (below) is dropped.
                    let external_domains_caseless = &self.external_domains_caseless;

                    // Weighted upgrade score: avoids Chrome on a single weak signal.
                    // Strong signals (framework markers, hydration IDs) set the score
                    // above the threshold immediately. Weak signals (script src) accumulate.
                    const SMART_UPGRADE_THRESHOLD: u8 = 10;
                    let upgrade_score = std::sync::atomic::AtomicU8::new(0);

                    let mut static_app = false;
                    let mut script_src_count: u8 = 0;
                    let xml_file = self.get_url().ends_with(".xml");

                    let mut element_content_handlers = metadata_handlers(
                        &mut meta_title,
                        &mut meta_description,
                        &mut meta_og_image,
                    );

                    element_content_handlers.push(element_precompiled!(
                        compiled_base_element_selector(),
                        |el| {
                            if let Some(href) = el.get_attribute("href") {
                                if let Ok(parsed_base) = Url::parse(&href) {
                                    let _ = base_input_url.set(parsed_base);
                                }
                            }

                            Ok(())
                        }
                    ));

                    element_content_handlers.push(element!("script", |el| {
                        if static_app
                            || upgrade_score.load(Ordering::Relaxed) >= SMART_UPGRADE_THRESHOLD
                        {
                            return Ok(());
                        }

                        let id = el.get_attribute("id");

                        if id.as_deref() == *NUXT_DATA {
                            static_app = true;
                            upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            return Ok(());
                        }

                        if el.get_attribute("data-target").as_deref() == *REACT_SSR {
                            upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            return Ok(());
                        }

                        let Some(src) = el.get_attribute("src") else {
                            return Ok(());
                        };

                        // Skip known ad/tracker scripts — they don't indicate an SPA.
                        if !is_tracker_script(&src) {
                            script_src_count = script_src_count.saturating_add(1);
                            if script_src_count >= 4 {
                                let _ = upgrade_score.fetch_update(
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                    |v| Some(v.saturating_add(SMART_UPGRADE_THRESHOLD)),
                                );
                            }
                        }

                        if !src.starts_with('/') {
                            return Ok(());
                        }

                        let is_next = src.starts_with("/_next/static/chunks/pages/")
                            || src.starts_with("/webpack-runtime-");
                        let is_gatsby = id.as_deref() == *GATSBY;

                        let is_nuxt_asset = src.starts_with("/_nuxt/");

                        if is_next || is_gatsby || is_nuxt_asset {
                            static_app = true;
                        }

                        if is_nuxt_asset {
                            upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            return Ok(());
                        }

                        if let Some(base) = base1.as_ref() {
                            let abs = convert_abs_path(base, &src);

                            if abs.path_segments().is_some_and(|mut segs| {
                                segs.any(|p| {
                                    chromiumoxide::handler::network::ALLOWED_MATCHER.is_match(p)
                                })
                            }) {
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            }
                        }

                        Ok(())
                    }));

                    element_content_handlers.push(element_precompiled!(
                        if xml_file {
                            compiled_xml_selector()
                        } else {
                            compiled_selector()
                        },
                        |el| {
                            if let Some(href) = el.get_attribute("href") {
                                let base = if relative_directory_url(&href) || base.is_none() {
                                    original_page.as_ref()
                                } else {
                                    base.as_deref()
                                };

                                let base = if base_input_url.initialized() {
                                    base_input_url.get()
                                } else {
                                    base
                                };

                                push_link(
                                    &base,
                                    &href,
                                    &mut inner_map,
                                    &selectors.0,
                                    parent_host,
                                    parent_host_scheme,
                                    base_input_domain,
                                    sub_matcher,
                                    external_domains_caseless,
                                    &mut links_pages,
                                );
                            }

                            Ok(())
                        }
                    ));

                    element_content_handlers.push(text!("noscript", |el| {
                        if upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD {
                            if NO_SCRIPT_JS_REQUIRED.find(el.as_str()).is_some() {
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            }
                        }
                        Ok(())
                    }));

                    element_content_handlers.push(text!("script", |el| {
                        let s = el.as_str();
                        if !s.is_empty()
                            && upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD
                        {
                            if DOM_SCRIPT_WATCH_METHODS.find(s).is_some() {
                                // Inline DOM mutation is a medium signal (7 points).
                                // Combined with script srcs it crosses the threshold.
                                let _ = upgrade_score.fetch_update(
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                    |v| Some(v.saturating_add(7)),
                                );
                            }
                        }
                        Ok(())
                    }));

                    element_content_handlers.push(element!("body", |el| {
                        if upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD {
                            let mut matched = false;

                            if let Some(id) = el.get_attribute("id") {
                                if HYDRATION_IDS.contains(&id) {
                                    upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                    matched = true;
                                }
                            }

                            if !matched {
                                for attr in DOM_WATCH_ATTRIBUTE_PATTERNS.iter() {
                                    if el.has_attribute(attr) {
                                        upgrade_score
                                            .store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                        break;
                                    }
                                }
                            }
                        }
                        Ok(())
                    }));

                    let rewriter_settings = lol_html::Settings {
                        element_content_handlers,
                        adjust_charset_on_meta_tag: true,
                        ..lol_html::send::Settings::new_for_handler_types()
                    };

                    let mut rewriter =
                        lol_html::send::HtmlRewriter::new(rewriter_settings, |_c: &[u8]| {});

                    let mut wrote_error = false;
                    let should_yield = html_bytes_taken.len() > REWRITER_YIELD_THRESHOLD;

                    for (i, chunk) in html_bytes_taken.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
                        if rewriter.write(chunk).is_err() {
                            wrote_error = true;
                            break;
                        }
                        if should_yield
                            && i % REWRITER_YIELD_INTERVAL == REWRITER_YIELD_INTERVAL - 1
                        {
                            tokio::task::yield_now().await;
                        }
                    }

                    // Consume the rewriter in both branches so its closures
                    // release their borrows of `self.external_domains_caseless`,
                    // `upgrade_score`, `inner_map`, and the metadata slots
                    // before the Chrome upgrade path touches `&mut self` below.
                    if !wrote_error {
                        let _ = rewriter.end();
                    } else {
                        drop(rewriter);
                    }

                    // Anti-bot detection is a strong signal (immediate upgrade).
                    let mut score = upgrade_score.load(Ordering::Relaxed);
                    if score < SMART_UPGRADE_THRESHOLD
                        && crate::utils::detect_anti_bot_from_body(&html_bytes_taken).is_some()
                    {
                        score = SMART_UPGRADE_THRESHOLD;
                    }

                    if score >= SMART_UPGRADE_THRESHOLD {
                        if let Some(browser_controller) = browser
                            .get_or_init(|| {
                                crate::website::Website::setup_browser_base(
                                    &configuration,
                                    &base,
                                    jar,
                                )
                            })
                            .await
                        {
                            if let Ok(new_page) = crate::features::chrome::attempt_navigation(
                                "about:blank",
                                &browser_controller.browser.0,
                                &configuration.request_timeout,
                                &browser_controller.browser.2,
                                &configuration.viewport,
                            )
                            .await
                            {
                                let (intercept_handle, _) = tokio::join!(
                                    crate::features::chrome::setup_chrome_interception_base(
                                        &new_page,
                                        configuration.chrome_intercept.enabled,
                                        &configuration.auth_challenge_response,
                                        configuration.chrome_intercept.block_visuals,
                                        &parent_host,
                                    ),
                                    crate::features::chrome::setup_chrome_events(
                                        &new_page,
                                        &configuration,
                                    ),
                                );

                                if let Some(cookie_jar) = jar {
                                    if let Some(u) = &original_page {
                                        if !configuration.cookie_str.is_empty() {
                                            let _ =
                                            crate::features::chrome::seed_jar_from_cookie_header(
                                                cookie_jar,
                                                &configuration.cookie_str,
                                                &u,
                                            );
                                        }

                                        if let Ok(cps) =
                                            crate::features::chrome::cookie_params_from_jar(
                                                cookie_jar, &u,
                                            )
                                        {
                                            let _ = crate::features::chrome::set_page_cookies(
                                                &new_page, cps,
                                            )
                                            .await;
                                        }
                                    }
                                }

                                let fetch_params = configuration.chrome_fetch_params();

                                // Streaming-extraction setup for the chrome
                                // upgrade. lol_html walks the rendered body
                                // *during* the chrome chunk pump — same
                                // single-pass shape as the recursive chrome
                                // path. When the upgrade succeeds, we drop
                                // `inner_map` (HTTP-extracted links from the
                                // un-rendered body) and use chrome's links as
                                // authoritative; on streaming failure the
                                // legacy second-pass walk over the assembled
                                // body still runs, preserving prior behavior.
                                let chrome_parsed_target = original_page.clone();
                                let chrome_xml_file = self.get_url().ends_with(".xml");
                                let chrome_base_input_url = tokio::sync::OnceCell::new();
                                // Discarded sinks for metadata + page_links —
                                // the HTTP pre-pass already wrote to the
                                // outer `meta_*` / `links_pages`, and we
                                // preserve that legacy behavior. These exist
                                // only because `build_link_extract_handlers`
                                // requires the `&mut` slots.
                                let mut chrome_meta_title_unused: Option<CompactString> = None;
                                let mut chrome_meta_description_unused: Option<CompactString> =
                                    None;
                                let mut chrome_meta_og_image_unused: Option<CompactString> = None;
                                let mut chrome_links_pages_unused: Option<HashSet<A>> = None;
                                let mut chrome_extracted_links: HashSet<A> =
                                    HashSet::with_capacity(link_set_capacity());

                                let (page_resource, chrome_extract_succeeded) = {
                                    let chrome_external_domains_caseless =
                                        &self.external_domains_caseless;
                                    let chrome_handlers = build_link_extract_handlers(
                                        LinkExtractCtx {
                                            selectors,
                                            external_domains_caseless:
                                                chrome_external_domains_caseless,
                                            map: &mut chrome_extracted_links,
                                            links_pages: &mut chrome_links_pages_unused,
                                            base_input_url: &chrome_base_input_url,
                                            base: chrome_parsed_target.as_ref(),
                                            original_page: chrome_parsed_target.as_ref(),
                                            ssg_raw_src_cell: None,
                                            ssg_resolved_path_cell: None,
                                            xml_file: chrome_xml_file,
                                            full_resources: false,
                                            skip_links: false,
                                        },
                                        &mut chrome_meta_title_unused,
                                        &mut chrome_meta_description_unused,
                                        &mut chrome_meta_og_image_unused,
                                    );

                                    let mut chrome_extract =
                                        ChromeStreamingExtractor::new(chrome_handlers, None, true);
                                    let resource = crate::utils::fetch_page_html_chrome_base(
                                        &html_bytes_taken,
                                        &new_page,
                                        true,
                                        true,
                                        false,
                                        Some(&self.url),
                                        configuration.referer.clone(),
                                        configuration.max_page_bytes,
                                        configuration.get_cache_options(),
                                        {
                                            #[cfg(feature = "headers")]
                                            {
                                                &self.headers
                                            }
                                            #[cfg(not(feature = "headers"))]
                                            {
                                                &None
                                            }
                                        },
                                        &Some(&configuration.chrome_intercept),
                                        jar,
                                        configuration.cache_namespace_str(),
                                        &fetch_params,
                                        Some(&mut chrome_extract),
                                    )
                                    .await;
                                    let succeeded = chrome_extract.end();
                                    (resource, succeeded)
                                };

                                if let Some(h) = intercept_handle {
                                    let abort_handle = h.abort_handle();
                                    if let Err(elasped) = tokio::time::timeout(
                                        tokio::time::Duration::from_secs(15),
                                        h,
                                    )
                                    .await
                                    {
                                        log::warn!("Handler timeout exceeded {elasped}");
                                        abort_handle.abort();
                                    }
                                }

                                if let Ok(resource) = page_resource {
                                    let base = if base_input_url.initialized() {
                                        base_input_url.get().cloned().map(Box::new)
                                    } else {
                                        base1.as_deref().cloned().map(Box::new)
                                    };

                                    bytes_transferred = resource.bytes_transferred;

                                    let new_page = build(&self.url, resource);

                                    page_assign(self, new_page);

                                    // Behavior parity with the legacy
                                    // `links_stream_base` second-pass over the
                                    // rendered body. When the streaming
                                    // extractor walked the body cleanly during
                                    // the chunk pump we use its link set
                                    // directly (single-pass perf win); on
                                    // streaming decline (XML carve-out,
                                    // mid-stream rewriter error) the second
                                    // walk runs as before. Either way the
                                    // outer `map.extend(inner_map)` below
                                    // still merges the HTTP pre-pass links —
                                    // bit-identical final link set vs prior
                                    // releases.
                                    let extended_map: HashSet<A> = if chrome_extract_succeeded {
                                        chrome_extracted_links
                                    } else {
                                        // `take` the html bytes to release
                                        // the immutable borrow on `self.html`
                                        // before `links_stream_base`
                                        // reborrows `self` mutably.
                                        let fallback_bytes = self.html.take();
                                        let m = self
                                            .links_stream_base::<A>(
                                                selectors,
                                                fallback_bytes.as_deref().unwrap_or(&[]),
                                                &base,
                                            )
                                            .await;
                                        if let Some(b) = fallback_bytes {
                                            self.html = Some(b);
                                        }
                                        m
                                    };

                                    map.extend(extended_map);
                                };
                            }
                        }
                    }
                }

                map.extend(inner_map);
                self.html = Some(html_bytes_taken);
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
            page_links.extend(map.iter().map(|item| item.clone().into()));
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        (map, bytes_transferred)
    }

    /// Find all the links as a stream using string resource validation.
    #[cfg(all(
        not(feature = "decentralized"),
        feature = "full_resources",
        feature = "smart"
    ))]
    #[inline(always)]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn links_stream_smart<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        configuration: &crate::configuration::Configuration,
        base: &Option<Box<Url>>,
        browser: &crate::features::chrome::OnceBrowser,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    ) -> (HashSet<A>, Option<f64>) {
        use lol_html::{element, text};
        use std::sync::atomic::Ordering;

        let mut bytes_transferred: Option<f64> = None;
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut inner_map: HashSet<A> = HashSet::new();
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if self.is_xml {
            if let Some(html_bytes_taken) = self.html.take() {
                self.links_stream_xml_links_stream_base(
                    selectors,
                    html_bytes_taken.as_ref(),
                    &mut map,
                    base,
                )
                .await;
                self.html = Some(html_bytes_taken);
            } else {
                #[cfg(all(feature = "balance", not(feature = "decentralized")))]
                if let Some(ref guard) = self.html_spool_path {
                    if let Some(path) = guard.path() {
                        self.links_stream_xml_from_disk(
                            selectors,
                            path.to_path_buf(),
                            &mut map,
                            base,
                        )
                        .await;
                    }
                }
            }
        } else {
            // When HTML is on disk (spooled), stream from spool file.
            #[cfg(all(feature = "balance", not(feature = "decentralized")))]
            if self.html.is_none() && self.html_spool_path.is_some() {
                if let Some(ref guard) = self.html_spool_path {
                    if let Some(path) = guard.path() {
                        let disk_links: HashSet<A> = self
                            .links_stream_base_from_disk(selectors, path.to_path_buf(), base)
                            .await;
                        map.extend(disk_links);
                    }
                }
            }

            if let Some(html_bytes_taken) = self.html.take() {
                {
                    let base_input_url = tokio::sync::OnceCell::new();

                    let base_input_domain = &selectors.2;
                    let parent_frags = &selectors.1; // todo: allow mix match tpt
                    let parent_host = &parent_frags[0];
                    let parent_host_scheme = &parent_frags[1];
                    let sub_matcher = &selectors.0;

                    let base1 = base.as_deref();

                    // original domain to match local pages.
                    let original_page = {
                        self.set_url_parsed_direct_empty();
                        self.get_url_parsed_ref().as_ref().cloned()
                    };

                    // Borrow the shared Arc rather than cloning — the borrow
                    // is released when the rewriter (below) is dropped.
                    let external_domains_caseless = &self.external_domains_caseless;

                    const SMART_UPGRADE_THRESHOLD: u8 = 10;
                    let upgrade_score = std::sync::atomic::AtomicU8::new(0);

                    let mut static_app = false;
                    let mut script_src_count: u8 = 0;

                    let mut element_content_handlers = vec![
                        element_precompiled!(compiled_base_element_selector(), |el| {
                            if let Some(href) = el.get_attribute("href") {
                                if let Ok(parsed_base) = Url::parse(&href) {
                                    let _ = base_input_url.set(parsed_base);
                                }
                            }

                            Ok(())
                        }),
                        element!("script", |el| {
                            if static_app
                                || upgrade_score.load(Ordering::Relaxed) >= SMART_UPGRADE_THRESHOLD
                            {
                                return Ok(());
                            }

                            let id = el.get_attribute("id");

                            if id.as_deref() == *NUXT_DATA {
                                static_app = true;
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                return Ok(());
                            }

                            if el.get_attribute("data-target").as_deref() == *REACT_SSR {
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                return Ok(());
                            }

                            let Some(src) = el.get_attribute("src") else {
                                return Ok(());
                            };

                            if !src.starts_with('/') {
                                return Ok(());
                            }

                            let is_next = src.starts_with("/_next/static/chunks/pages/")
                                || src.starts_with("/webpack-runtime-");
                            let is_gatsby = id.as_deref() == *GATSBY;

                            let is_nuxt_asset = src.starts_with("/_nuxt/");

                            if is_next || is_gatsby || is_nuxt_asset {
                                static_app = true;
                            }

                            if is_nuxt_asset {
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                return Ok(());
                            }

                            if let Some(base) = base1.as_ref() {
                                let abs = convert_abs_path(base, &src);

                                if abs.path_segments().is_some_and(|mut segs| {
                                    segs.any(|p| {
                                        chromiumoxide::handler::network::ALLOWED_MATCHER.is_match(p)
                                    })
                                }) {
                                    upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                }
                            }

                            Ok(())
                        }),
                        element!(
                            "a[href]:not([aria-hidden=\"true\"]),script[src],link[href]",
                            |el| {
                                let attribute = if el.tag_name() == "script" {
                                    if let Some(src) = el.get_attribute("src") {
                                        if !is_tracker_script(&src) {
                                            script_src_count = script_src_count.saturating_add(1);
                                            if script_src_count >= 4 {
                                                let _ = upgrade_score.fetch_update(
                                                    Ordering::Relaxed,
                                                    Ordering::Relaxed,
                                                    |v| {
                                                        Some(v.saturating_add(
                                                            SMART_UPGRADE_THRESHOLD,
                                                        ))
                                                    },
                                                );
                                            }
                                        }
                                    }
                                    "src"
                                } else {
                                    "href"
                                };
                                if let Some(href) = el.get_attribute(attribute) {
                                    let base = if relative_directory_url(&href) || base.is_none() {
                                        original_page.as_ref()
                                    } else {
                                        base.as_deref()
                                    };

                                    let base = if base_input_url.initialized() {
                                        base_input_url.get()
                                    } else {
                                        base
                                    };

                                    push_link(
                                        &base,
                                        &href,
                                        &mut inner_map,
                                        &selectors.0,
                                        parent_host,
                                        parent_host_scheme,
                                        base_input_domain,
                                        sub_matcher,
                                        external_domains_caseless,
                                        &mut links_pages,
                                    );
                                }

                                Ok(())
                            }
                        ),
                        text!("noscript", |el| {
                            if upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD
                                && NO_SCRIPT_JS_REQUIRED.find(el.as_str()).is_some()
                            {
                                upgrade_score.store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                            }
                            Ok(())
                        }),
                        text!("script", |el| {
                            let s = el.as_str();
                            if !s.is_empty()
                                && upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD
                                && DOM_SCRIPT_WATCH_METHODS.find(s).is_some()
                            {
                                let _ = upgrade_score.fetch_update(
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                    |v| Some(v.saturating_add(7)),
                                );
                            }
                            Ok(())
                        }),
                        element!("body", |el| {
                            if upgrade_score.load(Ordering::Relaxed) < SMART_UPGRADE_THRESHOLD {
                                let mut matched = false;

                                if let Some(id) = el.get_attribute("id") {
                                    if HYDRATION_IDS.contains(&id) {
                                        upgrade_score
                                            .store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                        matched = true;
                                    }
                                }

                                if !matched {
                                    for attr in DOM_WATCH_ATTRIBUTE_PATTERNS.iter() {
                                        if el.has_attribute(attr) {
                                            upgrade_score
                                                .store(SMART_UPGRADE_THRESHOLD, Ordering::Relaxed);
                                            break;
                                        }
                                    }
                                }
                            }
                            Ok(())
                        }),
                    ];

                    element_content_handlers.extend(metadata_handlers(
                        &mut meta_title,
                        &mut meta_description,
                        &mut meta_og_image,
                    ));

                    let rewriter_settings = lol_html::Settings {
                        element_content_handlers,
                        adjust_charset_on_meta_tag: true,
                        ..lol_html::send::Settings::new_for_handler_types()
                    };

                    let mut rewriter =
                        lol_html::send::HtmlRewriter::new(rewriter_settings, |_c: &[u8]| {});

                    let mut wrote_error = false;
                    let should_yield = html_bytes_taken.len() > REWRITER_YIELD_THRESHOLD;

                    for (i, chunk) in html_bytes_taken.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
                        if rewriter.write(chunk).is_err() {
                            wrote_error = true;
                            break;
                        }
                        if should_yield
                            && i % REWRITER_YIELD_INTERVAL == REWRITER_YIELD_INTERVAL - 1
                        {
                            tokio::task::yield_now().await;
                        }
                    }

                    // Consume the rewriter in both branches so its closures
                    // release their borrows of `self.external_domains_caseless`,
                    // `upgrade_score`, `inner_map`, and the metadata slots
                    // before the Chrome upgrade path touches `&mut self` below.
                    if !wrote_error {
                        let _ = rewriter.end();
                    } else {
                        drop(rewriter);
                    }

                    // Anti-bot detection is a strong signal (immediate upgrade).
                    let mut score = upgrade_score.load(Ordering::Relaxed);
                    if score < SMART_UPGRADE_THRESHOLD
                        && crate::utils::detect_anti_bot_from_body(&html_bytes_taken).is_some()
                    {
                        score = SMART_UPGRADE_THRESHOLD;
                    }

                    if score >= SMART_UPGRADE_THRESHOLD {
                        if let Some(browser_controller) = browser
                            .get_or_init(|| {
                                crate::website::Website::setup_browser_base(
                                    configuration,
                                    base,
                                    jar,
                                )
                            })
                            .await
                        {
                            if let Ok(new_page) = crate::features::chrome::attempt_navigation(
                                "about:blank",
                                &browser_controller.browser.0,
                                &configuration.request_timeout,
                                &browser_controller.browser.2,
                                &configuration.viewport,
                            )
                            .await
                            {
                                let (intercept_handle, _) = tokio::join!(
                                    crate::features::chrome::setup_chrome_interception_base(
                                        &new_page,
                                        configuration.chrome_intercept.enabled,
                                        &configuration.auth_challenge_response,
                                        configuration.chrome_intercept.block_visuals,
                                        parent_host,
                                    ),
                                    crate::features::chrome::setup_chrome_events(
                                        &new_page,
                                        configuration,
                                    )
                                );

                                if let Some(cookie_jar) = jar {
                                    if let Some(u) = &original_page {
                                        if !configuration.cookie_str.is_empty() {
                                            let _ =
                                            crate::features::chrome::seed_jar_from_cookie_header(
                                                cookie_jar,
                                                &configuration.cookie_str,
                                                u,
                                            );
                                        }

                                        if let Ok(cps) =
                                            crate::features::chrome::cookie_params_from_jar(
                                                cookie_jar, u,
                                            )
                                        {
                                            let _ = crate::features::chrome::set_page_cookies(
                                                &new_page, cps,
                                            )
                                            .await;
                                        }
                                    }
                                }

                                let fetch_params = configuration.chrome_fetch_params();

                                // Streaming-extraction setup for chrome
                                // upgrade — see the non-`full_resources`
                                // variant of `links_stream_smart` for the
                                // full rationale. Same single-pass shape:
                                // lol_html walks the rendered body during
                                // chrome chunk pump; on success drop the
                                // HTTP-pre-pass `inner_map` and use
                                // chrome's links as authoritative.
                                let chrome_parsed_target = original_page.clone();
                                let chrome_xml_file = self.get_url().ends_with(".xml");
                                let chrome_base_input_url = tokio::sync::OnceCell::new();
                                // Discarded sinks for metadata + page_links —
                                // see the non-`full_resources` variant for
                                // the rationale.
                                let mut chrome_meta_title_unused: Option<CompactString> = None;
                                let mut chrome_meta_description_unused: Option<CompactString> =
                                    None;
                                let mut chrome_meta_og_image_unused: Option<CompactString> = None;
                                let mut chrome_links_pages_unused: Option<HashSet<A>> = None;
                                let mut chrome_extracted_links: HashSet<A> =
                                    HashSet::with_capacity(link_set_capacity());

                                let (page_resource, chrome_extract_succeeded) = {
                                    let chrome_external_domains_caseless =
                                        &self.external_domains_caseless;
                                    let chrome_handlers = build_link_extract_handlers(
                                        LinkExtractCtx {
                                            selectors,
                                            external_domains_caseless:
                                                chrome_external_domains_caseless,
                                            map: &mut chrome_extracted_links,
                                            links_pages: &mut chrome_links_pages_unused,
                                            base_input_url: &chrome_base_input_url,
                                            base: chrome_parsed_target.as_ref(),
                                            original_page: chrome_parsed_target.as_ref(),
                                            ssg_raw_src_cell: None,
                                            ssg_resolved_path_cell: None,
                                            xml_file: chrome_xml_file,
                                            full_resources: true,
                                            skip_links: false,
                                        },
                                        &mut chrome_meta_title_unused,
                                        &mut chrome_meta_description_unused,
                                        &mut chrome_meta_og_image_unused,
                                    );

                                    let mut chrome_extract =
                                        ChromeStreamingExtractor::new(chrome_handlers, None, true);
                                    let resource = crate::utils::fetch_page_html_chrome_base(
                                        &html_bytes_taken,
                                        &new_page,
                                        true,
                                        true,
                                        false,
                                        Some(&self.url),
                                        configuration.referer.clone(),
                                        configuration.max_page_bytes,
                                        configuration.get_cache_options(),
                                        {
                                            #[cfg(feature = "headers")]
                                            {
                                                &self.headers
                                            }
                                            #[cfg(not(feature = "headers"))]
                                            {
                                                &None
                                            }
                                        },
                                        &Some(&configuration.chrome_intercept),
                                        jar,
                                        configuration.cache_namespace_str(),
                                        &fetch_params,
                                        Some(&mut chrome_extract),
                                    )
                                    .await;
                                    let succeeded = chrome_extract.end();
                                    (resource, succeeded)
                                };

                                if let Some(h) = intercept_handle {
                                    let abort_handle = h.abort_handle();
                                    if let Err(elasped) = tokio::time::timeout(
                                        tokio::time::Duration::from_secs(15),
                                        h,
                                    )
                                    .await
                                    {
                                        log::warn!("Handler timeout exceeded {elasped}");
                                        abort_handle.abort();
                                    }
                                }

                                if let Ok(v) = page_resource {
                                    bytes_transferred = v.bytes_transferred;
                                    let new_page = build(&self.url, v);
                                    page_assign(self, new_page);

                                    // Behavior parity with the legacy
                                    // `links_stream_base` second-pass — see
                                    // the non-`full_resources` variant for
                                    // the full rationale. Streaming-extracted
                                    // links replace the post-fetch walk on
                                    // success; legacy walk runs on decline.
                                    // Outer `map.extend(inner_map)` still
                                    // merges HTTP pre-pass links so the
                                    // final link set is bit-identical with
                                    // prior releases.
                                    let extended_map: HashSet<A> = if chrome_extract_succeeded {
                                        chrome_extracted_links
                                    } else {
                                        let fallback_bytes = self.html.take();
                                        let m = self
                                            .links_stream_base::<A>(
                                                selectors,
                                                fallback_bytes.as_deref().unwrap_or(&[]),
                                                &base.as_deref().cloned().map(Box::new),
                                            )
                                            .await;
                                        if let Some(b) = fallback_bytes {
                                            self.html = Some(b);
                                        }
                                        m
                                    };

                                    map.extend(extended_map);
                                }
                            }
                        }
                    }
                }

                map.extend(inner_map);
                self.html = Some(html_bytes_taken);
            }
        }

        if let Some(lp) = links_pages {
            let page_links = self.page_links.get_or_insert_with(Default::default);
            page_links.extend(lp.into_iter().map(Into::into));
            page_links.extend(map.iter().map(|item| item.clone().into()));
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        (map, bytes_transferred)
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all,))]
    pub async fn links_stream_full_resource<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        let mut map: HashSet<A> = HashSet::with_capacity(link_set_capacity());
        let mut links_pages: Option<HashSet<A>> = if self.page_links.is_some() {
            Some(HashSet::new())
        } else {
            None
        };

        let mut metadata: Option<Box<Metadata>> = None;
        let mut meta_title: Option<_> = None;
        let mut meta_description: Option<_> = None;
        let mut meta_og_image: Option<_> = None;

        if self.is_xml {
            if let Some(html_bytes_taken) = self.html.take() {
                self.links_stream_xml_links_stream_base(
                    selectors,
                    html_bytes_taken.as_ref(),
                    &mut map,
                    base,
                )
                .await;
                self.html = Some(html_bytes_taken);
            } else {
                #[cfg(all(feature = "balance", not(feature = "decentralized")))]
                if let Some(ref guard) = self.html_spool_path {
                    if let Some(path) = guard.path() {
                        self.links_stream_xml_from_disk(
                            selectors,
                            path.to_path_buf(),
                            &mut map,
                            base,
                        )
                        .await;
                    }
                }
            }
        } else {
            // When HTML is on disk (spooled), stream from spool file using
            // the base disk extractor instead of loading full content.
            #[cfg(all(feature = "balance", not(feature = "decentralized")))]
            if self.html.is_none() && self.html_spool_path.is_some() {
                if let Some(ref guard) = self.html_spool_path {
                    if let Some(path) = guard.path() {
                        let disk_links = self
                            .links_stream_base_from_disk(selectors, path.to_path_buf(), base)
                            .await;
                        map.extend(disk_links);
                    }
                }
            }

            if let Some(html_bytes_taken) = self.html.take() {
                {
                    let base_input_url = tokio::sync::OnceCell::new();
                    let base = base.as_deref();
                    let xml_file = self.get_url().ends_with(".xml");

                    self.set_url_parsed_direct_empty();
                    let original_page = self.get_url_parsed_ref().as_ref();
                    // Borrow the shared Arc rather than cloning — the borrow
                    // is released at the end of this block when the rewriter
                    // drops, before the outer `self.html = Some(..)` write.
                    let external_domains_caseless = &self.external_domains_caseless;

                    let element_content_handlers = build_link_extract_handlers(
                        LinkExtractCtx {
                            selectors,
                            external_domains_caseless,
                            map: &mut map,
                            links_pages: &mut links_pages,
                            base_input_url: &base_input_url,
                            base,
                            original_page,
                            ssg_raw_src_cell: None,
                            ssg_resolved_path_cell: None,
                            xml_file,
                            full_resources: true,
                            skip_links: false,
                        },
                        &mut meta_title,
                        &mut meta_description,
                        &mut meta_og_image,
                    );

                    let settings = lol_html::send::Settings {
                        element_content_handlers,
                        adjust_charset_on_meta_tag: true,
                        ..lol_html::send::Settings::new_for_handler_types()
                    };

                    let mut rewriter = lol_html::send::HtmlRewriter::new(settings, |_c: &[u8]| {});

                    let mut wrote_error = false;
                    let should_yield = html_bytes_taken.len() > REWRITER_YIELD_THRESHOLD;

                    for (i, chunk) in html_bytes_taken.chunks(*STREAMING_CHUNK_SIZE).enumerate() {
                        if rewriter.write(chunk).is_err() {
                            wrote_error = true;
                            break;
                        }
                        if should_yield
                            && i % REWRITER_YIELD_INTERVAL == REWRITER_YIELD_INTERVAL - 1
                        {
                            tokio::task::yield_now().await;
                        }
                    }

                    if !wrote_error {
                        let _ = rewriter.end();
                    }
                }

                self.html = Some(html_bytes_taken);
            }
        }

        let valid_meta =
            meta_title.is_some() || meta_description.is_some() || meta_og_image.is_some();

        if valid_meta {
            let mut metadata_inner = Metadata::default();
            metadata_inner.title = meta_title;
            metadata_inner.description = meta_description;
            metadata_inner.image = meta_og_image;

            if metadata_inner.exist() {
                metadata.replace(Box::new(metadata_inner));
            }

            if metadata.is_some() {
                self.metadata = metadata;
            }
        }

        update_link_capacity_hint(map.len());

        map
    }

    /// Find the links as a stream using string resource validation
    #[inline(always)]
    #[cfg(all(not(feature = "decentralized"), feature = "full_resources"))]
    pub async fn links_stream<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<A> {
        if self.is_binary_spool_aware() {
            Default::default()
        } else {
            // When HTML is on disk, stream from disk without loading full
            // content into memory.
            #[cfg(all(feature = "balance", not(feature = "decentralized")))]
            if self.html.is_none() && self.html_spool_path.is_some() {
                if let Some(ref guard) = self.html_spool_path {
                    if let Some(path) = guard.path() {
                        return self
                            .links_stream_base_from_disk(selectors, path.to_path_buf(), base)
                            .await;
                    }
                }
                return Default::default();
            }
            self.links_stream_full_resource(selectors, base).await
        }
    }

    #[inline(always)]
    #[cfg(feature = "decentralized")]
    /// Find the links as a stream using string resource validation
    pub async fn links_stream<
        A: PartialEq
            + Eq
            + Sync
            + Send
            + Clone
            + Default
            + ToString
            + std::hash::Hash
            + From<String>
            + Into<CaseInsensitiveString>
            + for<'a> From<&'a str>,
    >(
        &mut self,
        _: &RelativeSelectors,
    ) -> HashSet<A> {
        Default::default()
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(not(feature = "decentralized"))]
    #[inline(always)]
    pub async fn links(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        let has_html = self.html.is_some();

        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        let has_html = has_html || self.html_spool_path.is_some();

        match has_html {
            false => Default::default(),
            true => {
                self.links_stream::<CaseInsensitiveString>(selectors, base)
                    .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors gathering all resources.
    #[inline(always)]
    #[cfg(not(feature = "decentralized"))]
    pub async fn links_full(
        &mut self,
        selectors: &RelativeSelectors,
        base: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        let has_html = self.html.is_some();

        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        let has_html = has_html || self.html_spool_path.is_some();

        match has_html {
            false => Default::default(),
            true => {
                // When HTML is on disk, stream link extraction directly
                // without loading the full content into memory.
                #[cfg(all(feature = "balance", not(feature = "decentralized")))]
                if self.html.is_none() && self.html_spool_path.is_some() {
                    if let Some(ref guard) = self.html_spool_path {
                        if let Some(path) = guard.path() {
                            return self
                                .links_stream_base_from_disk(selectors, path.to_path_buf(), base)
                                .await;
                        }
                    }
                    return Default::default();
                }
                if self.is_binary_spool_aware() {
                    return Default::default();
                }
                self.links_stream_full_resource::<CaseInsensitiveString>(selectors, base)
                    .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(all(not(feature = "decentralized"), feature = "smart"))]
    #[inline(always)]
    pub(crate) async fn smart_links(
        &mut self,
        selectors: &RelativeSelectors,
        configuration: &crate::configuration::Configuration,
        base: &Option<Box<Url>>,
        page: &crate::features::chrome::OnceBrowser,
        jar: Option<&std::sync::Arc<crate::client::cookie::Jar>>,
    ) -> (HashSet<CaseInsensitiveString>, Option<f64>) {
        let has_html = self.html.is_some();

        #[cfg(all(feature = "balance", not(feature = "decentralized")))]
        let has_html = has_html || self.html_spool_path.is_some();

        match has_html {
            false => Default::default(),
            true => {
                // When HTML is on disk, stream link extraction directly
                // without loading the full content into memory.  The smart
                // Chrome-upgrade heuristic is skipped for disk pages — they
                // already have rendered content.
                #[cfg(all(feature = "balance", not(feature = "decentralized")))]
                if self.html.is_none() && self.html_spool_path.is_some() {
                    if let Some(ref guard) = self.html_spool_path {
                        if let Some(path) = guard.path() {
                            let links = self
                                .links_stream_base_from_disk(selectors, path.to_path_buf(), base)
                                .await;
                            return (links, None);
                        }
                    }
                    return Default::default();
                }
                if self.is_binary_spool_aware() {
                    return Default::default();
                }
                self.links_stream_smart::<CaseInsensitiveString>(
                    selectors,
                    configuration,
                    base,
                    page,
                    jar,
                )
                .await
            }
        }
    }

    /// Find all href links and return them using CSS selectors.
    #[cfg(feature = "decentralized")]
    #[inline(always)]
    pub async fn links(
        &self,
        _: &RelativeSelectors,
        _: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        self.links.to_owned()
    }

    /// Find all href links and return them using CSS selectors gathering all resources.
    #[cfg(feature = "decentralized")]
    #[inline(always)]
    pub async fn links_full(
        &self,
        _: &RelativeSelectors,
        _: &Option<Box<Url>>,
    ) -> HashSet<CaseInsensitiveString> {
        self.links.to_owned()
    }
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
pub fn encode_bytes(html: &[u8], label: &str) -> String {
    auto_encoder::encode_bytes(html, label)
}

/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
#[cfg(feature = "encoding")]
pub fn get_html_encoded(html: &Option<bytes::Bytes>, label: &str) -> String {
    match html.as_ref() {
        Some(html) => encode_bytes(html, label),
        _ => Default::default(),
    }
}

#[cfg(not(feature = "encoding"))]
/// Get the content with proper encoding. Pass in a proper encoding label like SHIFT_JIS.
pub fn get_html_encoded(html: &Option<bytes::Bytes>, _label: &str) -> String {
    match html {
        Some(b) => String::from_utf8_lossy(b).into_owned(),
        _ => Default::default(),
    }
}

#[cfg(all(test, not(feature = "decentralized"), feature = "smart"))]
mod smart_tests {
    use super::is_tracker_script;

    #[test]
    fn tracker_absolute_urls() {
        // Known tracker scripts (absolute) should be detected.
        assert!(is_tracker_script(
            "https://www.googletagmanager.com/gtm.js?id=GTM-ABC"
        ));
        assert!(is_tracker_script(
            "https://www.google-analytics.com/analytics.js"
        ));
        assert!(is_tracker_script(
            "https://static.hotjar.com/c/hotjar-123.js"
        ));
        assert!(is_tracker_script(
            "https://connect.facebook.net/en_US/fbevents.js"
        ));
    }

    #[test]
    fn non_tracker_absolute_urls() {
        // Application scripts should not be flagged.
        assert!(!is_tracker_script("https://cdn.example.com/app.js"));
        assert!(!is_tracker_script(
            "https://unpkg.com/react@18/umd/react.production.min.js"
        ));
    }

    #[test]
    fn tracker_relative_paths() {
        // Relative paths matching adblock patterns.
        assert!(is_tracker_script("/js/analytics.js"));
        assert!(is_tracker_script("/scripts/gtm.js?id=GTM-XYZ"));
    }

    #[test]
    fn non_tracker_relative_paths() {
        // Normal app scripts should pass through.
        assert!(!is_tracker_script("/assets/app.bundle.js"));
        assert!(!is_tracker_script("/_next/static/chunks/pages/index.js"));
        assert!(!is_tracker_script("/main.js"));
    }
}

#[cfg(test)]
/// The test user agent.
pub const TEST_AGENT_NAME: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[cfg(all(
    feature = "headers",
    not(feature = "decentralized"),
    not(feature = "cache_request"),
))]
#[tokio::test]
async fn test_headers() {
    use crate::utils::PageResponse;
    use reqwest::header::HeaderName;
    use reqwest::header::HeaderValue;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        HeaderName::from_static("server"),
        HeaderValue::from_static("GitHub.com"),
    );
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("text/html; charset=utf-8"),
    );

    let page = build(
        "https://choosealicense.com/",
        PageResponse {
            content: Some(b"<html></html>".to_vec()),
            headers: Some(headers),
            status_code: StatusCode::OK,
            ..Default::default()
        },
    );

    let headers = page.headers.clone().expect("There should be some headers");

    assert_eq!(
        headers
            .get(HeaderName::from_static("server"))
            .expect("There should be a server header value"),
        HeaderValue::from_static("GitHub.com")
    );

    assert_eq!(
        headers
            .get(HeaderName::from_static("content-type"))
            .expect("There should be a content-type value"),
        HeaderValue::from_static("text/html; charset=utf-8")
    );
}

#[tokio::test]
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
async fn parse_links() {
    use crate::utils::PageResponse;

    let link_result = "https://choosealicense.com/";
    let html = br#"<html><body><a href="/about/">About</a></body></html>"#;
    let mut page = build_with_parse(
        link_result,
        PageResponse {
            content: Some(html.to_vec()),
            status_code: StatusCode::OK,
            ..Default::default()
        },
    );
    let selector = get_page_selectors(link_result, false, false);

    let links = page.links(&selector, &None).await;

    let about_page = "https://choosealicense.com/about/".into();

    assert!(
        links.contains::<CaseInsensitiveString>(&about_page),
        "Could not find {}. Theses URLs was found {:?}",
        about_page,
        &links
    );
}

#[tokio::test]
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
async fn test_status_code() {
    use crate::utils::PageResponse;
    let page = build(
        "https://choosealicense.com/does-not-exist",
        PageResponse {
            status_code: StatusCode::NOT_FOUND,
            ..Default::default()
        },
    );

    assert_eq!(page.status_code.as_u16(), 404);
}

#[tokio::test]
#[cfg(all(feature = "time", not(feature = "decentralized")))]
async fn test_duration() {
    let client = Client::default();
    let link_result = "https://choosealicense.com/";
    let page: Page = Page::new_page(link_result, &client).await;
    let duration_elasped = page.get_duration_elapsed().as_millis();

    assert!(
        duration_elasped < 6000,
        "Duration took longer than expected {}.",
        duration_elasped,
    );
}

// ============================================================================
// Metadata Tests
// ============================================================================

/// Test that Metadata::exist returns false when all fields are None.
#[test]
fn test_metadata_exist_empty() {
    let metadata = Metadata::default();
    assert!(!metadata.exist(), "Empty metadata should not exist");
}

/// Test that Metadata::exist returns true when title is set.
#[test]
fn test_metadata_exist_with_title() {
    let metadata = Metadata {
        title: Some(CompactString::from("Test Title")),
        ..Default::default()
    };
    assert!(metadata.exist(), "Metadata with title should exist");
}

/// Test that Metadata::exist returns true when description is set.
#[test]
fn test_metadata_exist_with_description() {
    let metadata = Metadata {
        description: Some(CompactString::from("Test Description")),
        ..Default::default()
    };
    assert!(metadata.exist(), "Metadata with description should exist");
}

/// Test that Metadata::exist returns true when image is set.
#[test]
fn test_metadata_exist_with_image() {
    let metadata = Metadata {
        image: Some(CompactString::from("https://example.com/image.png")),
        ..Default::default()
    };
    assert!(metadata.exist(), "Metadata with image should exist");
}

/// Test that Metadata::exist returns true when all fields are set.
#[test]
fn test_metadata_exist_all_fields() {
    let metadata = Metadata {
        title: Some(CompactString::from("Test Title")),
        description: Some(CompactString::from("Test Description")),
        image: Some(CompactString::from("https://example.com/image.png")),
        #[cfg(feature = "chrome")]
        automation: None,
    };
    assert!(metadata.exist(), "Metadata with all fields should exist");
}

/// Test metadata extraction via build function with PageResponse.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_metadata_via_build() {
    use crate::utils::PageResponse;

    let metadata = Metadata {
        title: Some(CompactString::from("Build Test Title")),
        description: Some(CompactString::from("Build Test Description")),
        image: Some(CompactString::from("https://example.com/build-image.png")),
        #[cfg(feature = "chrome")]
        automation: None,
    };

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        metadata: Some(Box::new(metadata)),
        ..Default::default()
    };

    let page = build("https://example.com", page_response);
    let page_metadata = page.get_metadata();

    assert!(page_metadata.is_some(), "Page should have metadata");

    let meta = page_metadata.as_ref().unwrap();
    assert_eq!(
        meta.title.as_deref(),
        Some("Build Test Title"),
        "Title should match"
    );
    assert_eq!(
        meta.description.as_deref(),
        Some("Build Test Description"),
        "Description should match"
    );
    assert_eq!(
        meta.image.as_deref(),
        Some("https://example.com/build-image.png"),
        "Image should match"
    );
}

/// Test metadata extraction via build_with_parse function.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_metadata_via_build_with_parse() {
    use crate::utils::PageResponse;

    let metadata = Metadata {
        title: Some(CompactString::from("Parse Test Title")),
        description: Some(CompactString::from("Parse Test Description")),
        image: Some(CompactString::from("https://example.com/parse-image.png")),
        #[cfg(feature = "chrome")]
        automation: None,
    };

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        metadata: Some(Box::new(metadata)),
        ..Default::default()
    };

    let page = build_with_parse("https://example.com/page", page_response);
    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page should have metadata after build_with_parse"
    );

    let meta = page_metadata.as_ref().unwrap();
    assert_eq!(
        meta.title.as_deref(),
        Some("Parse Test Title"),
        "Title should match after build_with_parse"
    );
}

/// Test that Page without metadata returns None from get_metadata.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_page_without_metadata() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        metadata: None,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);
    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_none(),
        "Page without metadata should return None"
    );
}

/// Test metadata extraction using new_page_streaming_from_bytes with crafted HTML.
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_from_streaming_bytes() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <title>Streaming Test Title</title>
    <meta name="description" content="Streaming Test Description">
    <meta property="og:image" content="https://example.com/streaming-image.png">
</head>
<body>
    <a href="/page1">Link 1</a>
    <a href="/page2">Link 2</a>
</body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page from streaming bytes should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();

    assert_eq!(
        meta.title.as_deref(),
        Some("Streaming Test Title"),
        "Title should be extracted from streaming bytes"
    );
    assert_eq!(
        meta.description.as_deref(),
        Some("Streaming Test Description"),
        "Description should be extracted from streaming bytes"
    );
    assert_eq!(
        meta.image.as_deref(),
        Some("https://example.com/streaming-image.png"),
        "OG image should be extracted from streaming bytes"
    );
}

/// Test metadata extraction with partial metadata (only title).
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_partial_title_only() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <title>Only Title Here</title>
</head>
<body></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with only title should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();

    assert_eq!(
        meta.title.as_deref(),
        Some("Only Title Here"),
        "Title should be extracted"
    );
    assert!(meta.description.is_none(), "Description should be None");
    assert!(meta.image.is_none(), "Image should be None");
}

/// Test metadata extraction with partial metadata (only description).
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_partial_description_only() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <meta name="description" content="Only Description Here">
</head>
<body></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with only description should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();

    assert!(meta.title.is_none(), "Title should be None");
    assert_eq!(
        meta.description.as_deref(),
        Some("Only Description Here"),
        "Description should be extracted"
    );
    assert!(meta.image.is_none(), "Image should be None");
}

/// Test metadata extraction with partial metadata (only og:image).
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_partial_image_only() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <meta property="og:image" content="https://example.com/only-image.png">
</head>
<body></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with only og:image should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();

    assert!(meta.title.is_none(), "Title should be None");
    assert!(meta.description.is_none(), "Description should be None");
    assert_eq!(
        meta.image.as_deref(),
        Some("https://example.com/only-image.png"),
        "OG image should be extracted"
    );
}

/// Test that page with no metadata tags returns None.
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_empty_html() {
    let html = br#"<!DOCTYPE html>
<html>
<head></head>
<body><p>No metadata here</p></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_none(),
        "Page without any metadata tags should return None"
    );
}

/// Test metadata with special characters in content.
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_special_characters() {
    let html = br#"<!DOCTYPE html>
<html>
<head>
    <title>Title with &amp; special &lt;characters&gt;</title>
    <meta name="description" content="Description with &quot;quotes&quot; and 'apostrophes'">
    <meta property="og:image" content="https://example.com/image?param=value&amp;other=1">
</head>
<body></body>
</html>"#;

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with special characters should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();
    assert!(
        meta.title.is_some(),
        "Title with special chars should be extracted"
    );
    assert!(
        meta.description.is_some(),
        "Description with special chars should be extracted"
    );
    assert!(
        meta.image.is_some(),
        "Image URL with special chars should be extracted"
    );
}

/// Test metadata with unicode content.
#[tokio::test]
#[cfg(all(feature = "cmd", not(feature = "decentralized")))]
async fn test_metadata_unicode() {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>日本語タイトル - Japanese Title</title>
    <meta name="description" content="中文描述 - Chinese Description - Описание на русском">
    <meta property="og:image" content="https://example.com/画像.png">
</head>
<body></body>
</html>"#
        .as_bytes();

    let url = "https://example.com/test";
    let mut selectors = get_page_selectors(url, false, false);
    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Default::default();
    let r_settings = PageLinkBuildSettings::default();
    let mut map: HashSet<CaseInsensitiveString> = HashSet::with_capacity(32);
    let prior_domain: Option<Box<Url>> = None;
    let mut domain_parsed: Option<Box<Url>> = None;
    let mut links_pages: Option<HashSet<CaseInsensitiveString>> = None;

    let page = Page::new_page_streaming_from_bytes::<CaseInsensitiveString>(
        url,
        html,
        &mut selectors,
        &external_domains,
        &r_settings,
        &mut map,
        None,
        &prior_domain,
        &mut domain_parsed,
        &mut links_pages,
    )
    .await;

    let page_metadata = page.get_metadata();

    assert!(
        page_metadata.is_some(),
        "Page with unicode content should have metadata"
    );

    let meta = page_metadata.as_ref().unwrap();
    assert!(
        meta.title
            .as_ref()
            .map(|t| t.contains("日本語"))
            .unwrap_or(false),
        "Title should contain Japanese characters"
    );
    assert!(
        meta.description
            .as_ref()
            .map(|d| d.contains("中文"))
            .unwrap_or(false),
        "Description should contain Chinese characters"
    );
}

/// Test metadata with chrome feature - AutomationResults structure.
#[test]
#[cfg(feature = "chrome")]
fn test_automation_results_structure() {
    let automation_result = AutomationResults {
        input: "Test prompt".to_string(),
        content_output: serde_json::json!({"result": "test"}),
        screenshot_output: Some("base64_screenshot_data".to_string()),
        error: None,
        usage: None,
        relevant: None,
        steps_executed: None,
        reasoning: None,
    };

    assert_eq!(automation_result.input, "Test prompt");
    assert!(automation_result.screenshot_output.is_some());
    assert!(automation_result.error.is_none());
}

/// Test metadata with automation results (chrome feature).
#[test]
#[cfg(feature = "chrome")]
fn test_metadata_with_automation() {
    let automation_results = vec![AutomationResults {
        input: "Click the button".to_string(),
        content_output: serde_json::json!({"clicked": true}),
        screenshot_output: None,
        error: None,
        usage: None,
        relevant: None,
        steps_executed: None,
        reasoning: None,
    }];

    let metadata = Metadata {
        title: Some(CompactString::from("Automation Test")),
        description: None,
        image: None,
        automation: Some(automation_results),
    };

    assert!(metadata.exist(), "Metadata with title should exist");
    assert!(
        metadata.automation.is_some(),
        "Automation results should be present"
    );
    assert_eq!(
        metadata.automation.as_ref().unwrap().len(),
        1,
        "Should have one automation result"
    );
}

/// Test set_metadata function preserves automation results (chrome feature).
#[test]
#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
fn test_set_metadata_preserves_automation() {
    let automation_results = vec![AutomationResults {
        input: "Original automation".to_string(),
        content_output: serde_json::json!({"original": true}),
        screenshot_output: None,
        error: None,
        usage: None,
        relevant: None,
        steps_executed: None,
        reasoning: None,
    }];

    let existing_metadata = Metadata {
        title: Some(CompactString::from("Original Title")),
        description: None,
        image: None,
        automation: Some(automation_results),
    };

    let mut existing = Some(Box::new(existing_metadata));

    let mut new_metadata = Metadata {
        title: Some(CompactString::from("New Title")),
        description: Some(CompactString::from("New Description")),
        image: None,
        automation: None,
    };

    set_metadata(&mut existing, &mut new_metadata);

    assert!(
        new_metadata.automation.is_some(),
        "Automation should be preserved from existing metadata"
    );
}

/// Test metadata via Page::new with chrome feature.
#[tokio::test]
#[cfg(all(
    feature = "chrome",
    not(feature = "decentralized"),
    not(feature = "cache_request")
))]
async fn test_metadata_chrome_real_page() {
    use crate::utils::PageResponse;

    // Keep this deterministic: verify chrome-gated metadata plumbing without external network.
    let automation_results = vec![AutomationResults {
        input: "Extract CTA".to_string(),
        content_output: serde_json::json!({"cta": "Sign up"}),
        screenshot_output: Some("base64_screenshot_data".to_string()),
        error: None,
        usage: None,
        relevant: Some(true),
        steps_executed: Some(1),
        reasoning: Some("CTA extracted from main hero section".to_string()),
    }];

    let metadata = Metadata {
        title: Some(CompactString::from("Chrome Metadata Test")),
        description: Some(CompactString::from("Description available")),
        image: Some(CompactString::from("https://example.com/image.png")),
        automation: Some(automation_results),
    };

    let page = build(
        "https://example.com",
        PageResponse {
            content: Some(b"<html></html>".to_vec()),
            status_code: StatusCode::OK,
            metadata: Some(Box::new(metadata)),
            ..Default::default()
        },
    );

    let meta = page
        .get_metadata()
        .as_ref()
        .expect("metadata should be present for chrome feature test");
    assert!(meta.title.as_deref() == Some("Chrome Metadata Test"));
    assert!(
        meta.automation.is_some(),
        "automation metadata should be present"
    );
    assert_eq!(meta.automation.as_ref().expect("automation data").len(), 1);
}

// ============================================================================
// Feature-specific Tests
// ============================================================================

/// Test encoding feature - get_html_encoded function.
#[test]
#[cfg(feature = "encoding")]
fn test_encoding_get_html_encoded() {
    // Test with UTF-8 content
    let html_bytes = "こんにちは世界".as_bytes().to_vec();
    let encoded = encode_bytes(&html_bytes, "UTF-8");
    assert!(
        encoded.contains("こんにちは"),
        "UTF-8 encoding should preserve Japanese characters"
    );
}

/// Test encoding feature - get_html_encoded with Page.
#[test]
#[cfg(all(feature = "encoding", not(feature = "decentralized")))]
fn test_encoding_page_get_html_encoded() {
    use crate::utils::PageResponse;

    let html_content = "Hello World - テスト";
    let page_response = PageResponse {
        content: Some(html_content.as_bytes().to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);
    let encoded = page.get_html_encoded("UTF-8");

    assert!(
        encoded.contains("Hello World"),
        "Encoded content should contain ASCII text"
    );
    assert!(
        encoded.contains("テスト"),
        "Encoded content should contain Japanese text"
    );
}

/// Test remote_addr feature - Page struct has remote_addr field.
#[test]
#[cfg(all(feature = "remote_addr", not(feature = "decentralized")))]
fn test_remote_addr_field() {
    use crate::utils::PageResponse;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        remote_addr: Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            8080,
        )),
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    assert!(
        page.remote_addr.is_some(),
        "Page should have remote_addr when feature is enabled"
    );

    let addr = page.remote_addr.unwrap();
    assert_eq!(addr.ip(), IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    assert_eq!(addr.port(), 8080);
}

/// Test page_error_status_details feature - error_status is Arc<reqwest::Error>.
#[test]
#[cfg(all(feature = "page_error_status_details", not(feature = "decentralized")))]
fn test_page_error_status_details() {
    use crate::utils::PageResponse;

    // When page_error_status_details is enabled, error_status should be Option<Arc<reqwest::Error>>
    let page_response = PageResponse {
        content: None,
        status_code: StatusCode::INTERNAL_SERVER_ERROR,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    // The error_status type should be Option<Arc<reqwest::Error>> with this feature
    // We just verify it compiles and can be accessed
    let _error: &Option<std::sync::Arc<reqwest::Error>> = &page.error_status;
}

/// Test page_error_status_details feature disabled - error_status is String.
#[test]
#[cfg(all(
    not(feature = "page_error_status_details"),
    not(feature = "decentralized")
))]
fn test_page_error_status_string() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: None,
        status_code: StatusCode::INTERNAL_SERVER_ERROR,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    // The error_status type should be Option<String> without the feature
    let _error: &Option<String> = &page.error_status;
}

/// Test cookies feature - Page struct has cookies field.
#[test]
#[cfg(all(feature = "cookies", not(feature = "decentralized")))]
fn test_cookies_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    // Verify cookies field exists and is accessible
    let _cookies: &Option<reqwest::header::HeaderMap> = &page.cookies;
}

/// Test chrome feature - screenshot_bytes field exists.
#[test]
#[cfg(all(feature = "chrome", not(feature = "decentralized")))]
fn test_chrome_screenshot_bytes_field() {
    use crate::utils::PageResponse;

    let screenshot_data = vec![0x89, 0x50, 0x4E, 0x47]; // PNG header bytes

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        screenshot_bytes: Some(screenshot_data.clone()),
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    assert!(
        page.screenshot_bytes.is_some(),
        "Page should have screenshot_bytes when chrome feature is enabled"
    );
    assert_eq!(
        page.screenshot_bytes.as_ref().unwrap(),
        &screenshot_data,
        "Screenshot bytes should match"
    );
}

/// Test time feature - duration field and get_duration_elapsed.
#[test]
#[cfg(all(feature = "time", not(feature = "decentralized")))]
fn test_time_duration_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        duration: Some(tokio::time::Instant::now()),
        ..Default::default()
    };

    let page = build("https://example.com", page_response);
    let duration = page.get_duration_elapsed();

    // Duration should be very small since we just created the page
    assert!(
        duration.as_millis() < 1000,
        "Duration should be less than 1 second"
    );
}

/// Test openai feature - Page struct has openai_credits_used and extra_ai_data fields.
#[test]
#[cfg(all(feature = "openai", not(feature = "decentralized")))]
fn test_openai_fields() {
    use crate::features::openai_common::OpenAIUsage;
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // Test openai_credits_used field
    assert!(
        page.openai_credits_used.is_none(),
        "openai_credits_used should be None initially"
    );

    // Set openai_credits_used
    page.openai_credits_used = Some(vec![OpenAIUsage::default()]);
    assert!(
        page.openai_credits_used.is_some(),
        "openai_credits_used should be set"
    );

    // Test extra_ai_data field
    assert!(
        page.extra_ai_data.is_none(),
        "extra_ai_data should be None initially"
    );

    // Set extra_ai_data
    page.extra_ai_data = Some(vec![AIResults::default()]);
    assert!(page.extra_ai_data.is_some(), "extra_ai_data should be set");
}

/// Test gemini feature - Page struct has gemini_credits_used and extra_gemini_data fields.
#[test]
#[cfg(all(feature = "gemini", not(feature = "decentralized")))]
fn test_gemini_fields() {
    use crate::features::gemini_common::GeminiUsage;
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // Test gemini_credits_used field
    assert!(
        page.gemini_credits_used.is_none(),
        "gemini_credits_used should be None initially"
    );

    // Set gemini_credits_used
    page.gemini_credits_used = Some(vec![GeminiUsage::default()]);
    assert!(
        page.gemini_credits_used.is_some(),
        "gemini_credits_used should be set"
    );

    // Test extra_gemini_data field
    assert!(
        page.extra_gemini_data.is_none(),
        "extra_gemini_data should be None initially"
    );

    // Set extra_gemini_data
    page.extra_gemini_data = Some(vec![AIResults::default()]);
    assert!(
        page.extra_gemini_data.is_some(),
        "extra_gemini_data should be set"
    );
}

/// Test serde feature - Metadata can be serialized and deserialized.
#[test]
#[cfg(feature = "serde")]
fn test_metadata_serde() {
    let metadata = Metadata {
        title: Some("Test Title".into()),
        description: Some("Test Description".into()),
        image: Some("https://example.com/image.png".into()),
        #[cfg(feature = "chrome")]
        automation: None,
    };

    // Serialize to JSON
    let json = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    assert!(json.contains("Test Title"), "JSON should contain title");

    // Deserialize from JSON
    let deserialized: Metadata =
        serde_json::from_str(&json).expect("Failed to deserialize metadata");
    assert_eq!(
        metadata.title, deserialized.title,
        "Title should match after deserialization"
    );
    assert_eq!(
        metadata.description, deserialized.description,
        "Description should match after deserialization"
    );
    assert_eq!(
        metadata.image, deserialized.image,
        "Image should match after deserialization"
    );
}

/// Test serde feature - AIResults can be serialized and deserialized.
#[test]
#[cfg(feature = "serde")]
fn test_airesults_serde() {
    let ai_results = AIResults {
        input: "Test prompt".to_string(),
        js_output: "console.log('test');".to_string(),
        content_output: vec!["Result 1".to_string(), "Result 2".to_string()],
        screenshot_output: None,
        error: None,
    };

    // Serialize to JSON
    let json = serde_json::to_string(&ai_results).expect("Failed to serialize AIResults");
    assert!(json.contains("Test prompt"), "JSON should contain input");

    // Deserialize from JSON
    let deserialized: AIResults =
        serde_json::from_str(&json).expect("Failed to deserialize AIResults");
    assert_eq!(
        ai_results.input, deserialized.input,
        "Input should match after deserialization"
    );
    assert_eq!(
        ai_results.js_output, deserialized.js_output,
        "JS output should match after deserialization"
    );
    assert_eq!(
        ai_results.content_output.len(),
        deserialized.content_output.len(),
        "Content output length should match"
    );
}

/// Test decentralized feature - Page struct has links field.
#[test]
#[cfg(feature = "decentralized")]
fn test_decentralized_page() {
    // Decentralized Page struct has a links field for distributed crawling
    let page = Page::default();

    // Default Page should have empty links
    assert!(
        page.links.is_empty(),
        "Default Page should have empty links"
    );

    // Decentralized Page has external_domains_caseless field
    assert!(
        page.external_domains_caseless.is_empty(),
        "Default Page should have empty external_domains_caseless"
    );
}

/// Test smart feature implies chrome and chrome_intercept.
#[test]
#[cfg(all(feature = "smart", not(feature = "decentralized")))]
fn test_smart_feature() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let page = build("https://example.com", page_response);

    // Smart feature includes chrome, so screenshot_bytes should exist
    assert!(
        page.screenshot_bytes.is_none(),
        "screenshot_bytes should be None initially"
    );
}

#[test]
#[cfg(not(feature = "decentralized"))]
fn test_build_preserves_spawn_pages() {
    use crate::utils::PageResponse;

    let page = build(
        "https://example.com",
        PageResponse {
            status_code: StatusCode::OK,
            spawn_pages: Some(vec![
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ]),
            ..Default::default()
        },
    );

    let spawn_pages = page
        .spawn_pages
        .as_ref()
        .expect("spawn_pages should be preserved");
    assert_eq!(spawn_pages.len(), 2);
    assert_eq!(spawn_pages[0], "https://example.com/a");
    assert_eq!(spawn_pages[1], "https://example.com/b");
}

#[test]
#[cfg(all(feature = "smart", not(feature = "decentralized")))]
fn test_page_assign_merges_spawn_pages() {
    use crate::utils::PageResponse;

    let mut page = build(
        "https://example.com",
        PageResponse {
            status_code: StatusCode::OK,
            spawn_pages: Some(vec!["https://example.com/root".to_string()]),
            ..Default::default()
        },
    );

    let new_page = build(
        "https://example.com",
        PageResponse {
            status_code: StatusCode::OK,
            spawn_pages: Some(vec![
                "https://example.com/x".to_string(),
                "https://example.com/y".to_string(),
            ]),
            ..Default::default()
        },
    );

    page_assign(&mut page, new_page);

    let spawn_pages = page
        .spawn_pages
        .as_ref()
        .expect("spawn_pages should be merged");
    assert_eq!(spawn_pages.len(), 3);
    assert!(spawn_pages.contains(&"https://example.com/root".to_string()));
    assert!(spawn_pages.contains(&"https://example.com/x".to_string()));
    assert!(spawn_pages.contains(&"https://example.com/y".to_string()));
}

/// Test page_links field exists and works correctly.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_page_links_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // page_links should be None initially
    assert!(
        page.page_links.is_none(),
        "page_links should be None initially"
    );

    // Set page_links
    let mut links = HashSet::new();
    links.insert(CaseInsensitiveString::new("https://example.com/page1"));
    page.page_links = Some(Box::new(links));

    assert!(page.page_links.is_some(), "page_links should be set");
    assert_eq!(
        page.page_links.as_ref().unwrap().len(),
        1,
        "page_links should have 1 link"
    );
}

/// Test bytes_transferred field exists and works correctly.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_bytes_transferred_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // bytes_transferred should be None initially
    assert!(
        page.bytes_transferred.is_none(),
        "bytes_transferred should be None initially"
    );

    // Set bytes_transferred
    page.bytes_transferred = Some(1024.0);
    assert_eq!(
        page.bytes_transferred,
        Some(1024.0),
        "bytes_transferred should be 1024.0"
    );
}

/// Test waf_check and should_retry fields exist and work correctly.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_waf_and_retry_fields() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // waf_check and should_retry should be false initially
    assert!(!page.waf_check, "waf_check should be false initially");
    assert!(!page.should_retry, "should_retry should be false initially");

    // Set waf_check
    page.waf_check = true;
    assert!(page.waf_check, "waf_check should be true");

    // Set should_retry
    page.should_retry = true;
    assert!(page.should_retry, "should_retry should be true");
}

/// Test that is_retryable_status excludes DNS_RESOLVE_ERROR but includes other 5xx codes.
#[test]
fn test_is_retryable_status_excludes_dns() {
    assert!(
        !is_retryable_status(*DNS_RESOLVE_ERROR),
        "DNS_RESOLVE_ERROR (525) must not be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR),
        "500 should be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::BAD_GATEWAY),
        "502 should be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::SERVICE_UNAVAILABLE),
        "503 should be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::GATEWAY_TIMEOUT),
        "504 should be retryable"
    );
    assert!(
        is_retryable_status(StatusCode::TOO_MANY_REQUESTS),
        "429 should be retryable"
    );
    assert!(
        !is_retryable_status(StatusCode::OK),
        "200 should not be retryable"
    );
    assert!(
        !is_retryable_status(*TOO_MANY_REDIRECTS_ERROR),
        "TOO_MANY_REDIRECTS_ERROR (310) must not be retryable — redirect loops are deterministic"
    );
}

/// Test blocked_crawl field exists and works correctly.
#[test]
#[cfg(not(feature = "decentralized"))]
fn test_blocked_crawl_field() {
    use crate::utils::PageResponse;

    let page_response = PageResponse {
        content: Some(b"<html></html>".to_vec()),
        status_code: StatusCode::OK,
        ..Default::default()
    };

    let mut page = build("https://example.com", page_response);

    // blocked_crawl should be false initially
    assert!(
        !page.blocked_crawl,
        "blocked_crawl should be false initially"
    );

    // Set blocked_crawl
    page.blocked_crawl = true;
    assert!(page.blocked_crawl, "blocked_crawl should be true");
}

/// Test extract_root_domain strips TLD correctly.
#[test]
fn test_extract_root_domain() {
    assert_eq!(extract_root_domain("example.com"), "example");
    assert_eq!(extract_root_domain("example.org"), "example");
    assert_eq!(extract_root_domain("sub.example.com"), "example.com");
    assert_eq!(extract_root_domain("deep.sub.example.co.uk"), "co.uk");
    assert_eq!(extract_root_domain("localhost"), "localhost");
}

/// Test is_subdomain matches across different TLDs.
#[test]
fn test_is_subdomain_tld_matching() {
    // Same root domain, different TLDs — should match (both 2-part → compare first part)
    assert!(is_subdomain("example.com", "example.org"));
    assert!(is_subdomain("example.net", "example.com"));

    // Both 3-part with same last two parts — should match
    assert!(is_subdomain("a.example.com", "b.example.com"));

    // 3-part vs 2-part extracts differently (example.com vs example) — won't match
    assert!(!is_subdomain("sub.example.com", "example.com"));

    // Different root domains — should NOT match
    assert!(!is_subdomain("example.com", "other.com"));
    assert!(!is_subdomain("myexample.com", "example.com"));
}

/// Test get_page_selectors_base with tld=true produces a root-domain matcher.
#[test]
fn test_get_page_selectors_base_tld() {
    let selectors = get_page_selectors_base("https://example.com/page", false, true);
    // First element is the sub_matcher — should be the root domain without TLD
    assert_eq!(selectors.0.as_str(), "example");

    let selectors_no_tld = get_page_selectors_base("https://example.com/page", false, false);
    // Without tld, sub_matcher should be empty
    assert!(selectors_no_tld.0.is_empty());
}

/// Test parent_host_match allows different TLDs when sub_matcher is root domain.
#[test]
fn test_parent_host_match_tld() {
    let parent_host = CompactString::from("example.com");
    let base_host = CompactString::from("example.com");
    // sub_matcher is "example" (what extract_root_domain returns for tld mode)
    let sub_matcher = CompactString::from("example");

    // Same host — always allowed
    assert!(parent_host_match(
        Some("example.com"),
        "example",
        &parent_host,
        &base_host,
        &sub_matcher,
    ));

    // Different TLD — allowed via is_subdomain through sub_matcher
    assert!(parent_host_match(
        Some("example.org"),
        "example",
        &parent_host,
        &base_host,
        &sub_matcher,
    ));

    // Completely different domain — NOT allowed
    assert!(!parent_host_match(
        Some("other.com"),
        "example",
        &parent_host,
        &base_host,
        &sub_matcher,
    ));
}

/// Test that validate_link resolves relative URLs against the subdomain host,
/// not the original crawl domain. This is the core fix for subdomain crawling.
#[test]
fn test_validate_link_subdomain_relative_resolution() {
    // Simulate: crawling www.example.com with subdomains=true,
    // currently on a page at sub.example.com
    let selectors = get_page_selectors("https://www.example.com/", true, false);

    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Arc::new(HashSet::new());

    // With the fix: base is the page's own URL (sub.example.com)
    let subdomain_base = url::Url::parse("https://sub.example.com/page").unwrap();
    let mut no_page_links: Option<HashSet<CaseInsensitiveString>> = None;

    let result = validate_link(
        &Some(&subdomain_base),
        "/about",
        &selectors.0,
        &selectors.1[0],
        &selectors.2,
        &selectors.0,
        &external_domains,
        &mut no_page_links,
    );

    assert!(
        result.is_some(),
        "Relative link on subdomain page should be accepted"
    );
    assert_eq!(
        result.unwrap().as_str(),
        "https://sub.example.com/about",
        "Relative link should resolve against subdomain host, not crawl origin"
    );

    // Before the fix: base was the crawl origin (www.example.com)
    // This would incorrectly resolve /about to www.example.com/about
    let crawl_origin_base = url::Url::parse("https://www.example.com/").unwrap();
    let mut no_page_links2: Option<HashSet<CaseInsensitiveString>> = None;

    let result_old = validate_link(
        &Some(&crawl_origin_base),
        "/about",
        &selectors.0,
        &selectors.1[0],
        &selectors.2,
        &selectors.0,
        &external_domains,
        &mut no_page_links2,
    );

    assert!(result_old.is_some());
    assert_eq!(
        result_old.unwrap().as_str(),
        "https://www.example.com/about",
        "With crawl origin as base, link resolves against wrong host"
    );
}

/// Test that validate_link still works correctly for same-domain pages
/// (no regression from the subdomain fix).
#[test]
fn test_validate_link_same_domain_resolution() {
    let selectors = get_page_selectors("https://www.example.com/", false, false);

    let external_domains: Arc<HashSet<CaseInsensitiveString>> = Arc::new(HashSet::new());

    let page_base = url::Url::parse("https://www.example.com/some-page").unwrap();
    let mut no_page_links: Option<HashSet<CaseInsensitiveString>> = None;

    let result = validate_link(
        &Some(&page_base),
        "/about",
        &selectors.0,
        &selectors.1[0],
        &selectors.2,
        &selectors.0,
        &external_domains,
        &mut no_page_links,
    );

    assert!(result.is_some());
    assert_eq!(
        result.unwrap().as_str(),
        "https://www.example.com/about",
        "Same-domain relative link should resolve correctly"
    );
}

/// Integration test: build a page at a subdomain URL with relative links,
/// verify links() resolves them against the subdomain.
#[tokio::test]
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
async fn test_subdomain_page_links_resolution() {
    use crate::utils::PageResponse;

    let html = br#"<html><body>
        <a href="/about">About</a>
        <a href="/contact">Contact</a>
        <a href="https://sub.example.com/absolute">Absolute</a>
    </body></html>"#;

    let mut page = build_with_parse(
        "https://sub.example.com/page",
        PageResponse {
            content: Some(html.to_vec()),
            status_code: reqwest::StatusCode::OK,
            ..Default::default()
        },
    );

    // Selectors for crawl origin www.example.com with subdomains=true
    let selectors = get_page_selectors("https://www.example.com/", true, false);

    // Simulate the fix: base is derived from the page's own URL
    let page_base = url::Url::parse("https://sub.example.com/page")
        .ok()
        .map(Box::new);

    let links = page.links(&selectors, &page_base).await;

    let expected_about: CaseInsensitiveString = "https://sub.example.com/about".into();
    let expected_contact: CaseInsensitiveString = "https://sub.example.com/contact".into();
    let expected_absolute: CaseInsensitiveString = "https://sub.example.com/absolute".into();
    let wrong_about: CaseInsensitiveString = "https://www.example.com/about".into();

    assert!(
        links.contains(&expected_about),
        "Relative /about should resolve to sub.example.com/about, got: {:?}",
        &links
    );
    assert!(
        links.contains(&expected_contact),
        "Relative /contact should resolve to sub.example.com/contact, got: {:?}",
        &links
    );
    assert!(
        links.contains(&expected_absolute),
        "Absolute link should be preserved, got: {:?}",
        &links
    );
    assert!(
        !links.contains(&wrong_about),
        "Links should NOT resolve against crawl origin www.example.com"
    );
}

/// Test that same-domain page links still resolve correctly after the fix.
#[tokio::test]
#[cfg(all(
    not(feature = "decentralized"),
    not(feature = "chrome"),
    not(feature = "cache_request")
))]
async fn test_same_domain_page_links_resolution() {
    use crate::utils::PageResponse;

    let html = br#"<html><body><a href="/about">About</a></body></html>"#;

    let mut page = build_with_parse(
        "https://www.example.com/page",
        PageResponse {
            content: Some(html.to_vec()),
            status_code: reqwest::StatusCode::OK,
            ..Default::default()
        },
    );

    let selectors = get_page_selectors("https://www.example.com/", false, false);

    // Base is the page's own URL (same domain as crawl origin)
    let page_base = url::Url::parse("https://www.example.com/page")
        .ok()
        .map(Box::new);

    let links = page.links(&selectors, &page_base).await;

    let expected: CaseInsensitiveString = "https://www.example.com/about".into();
    assert!(
        links.contains(&expected),
        "Same-domain relative link should resolve correctly, got: {:?}",
        &links
    );
}

/// DNS resolve error (525) should not be retried and needs_retry() must be false.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_dns_error_no_retry() {
    let res = PageResponse {
        status_code: StatusCode::from_u16(525).unwrap(),
        content: None,
        ..Default::default()
    };
    let page = build("https://nonexistent.invalid", res);
    assert!(
        !page.should_retry,
        "DNS resolve errors (525) must not be retried"
    );
    assert!(
        !page.needs_retry(),
        "DNS resolve errors (525) — needs_retry() must be false"
    );
}

/// Normal server errors should still be retried.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_server_error_still_retries() {
    let res = PageResponse {
        status_code: StatusCode::INTERNAL_SERVER_ERROR,
        content: Some(Default::default()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    assert!(page.should_retry, "500 errors should still be retried");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_detected_as_empty() {
    // Realistic Chrome error page: ~157KB with dino game CSS/JS, no </body>,
    // ends with loadTimeDataRaw JSON blob containing errorCode.
    let padding = "x".repeat(1000); // enough to exceed 500 byte minimum
    let chrome_error_html_str = format!(
        "<html lang=\"en\" dir=\"ltr\">\n\
         <style>{padding}</style>\n\
         <div id=\"main-frame-error\" class=\"interstitial-wrapper\">\n\
         <h1><span>This site can\u{2019}t be reached</span></h1>\n\
         <div class=\"error-code\">ERR_TUNNEL_CONNECTION_FAILED</div>\n\
         </div>\n\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_TUNNEL_CONNECTION_FAILED\",\
         \"heading\":{{\"msg\":\"This site can't be reached\"}},\
         \"title\":\"www.example.com\"}};</script></html>"
    );
    let chrome_error_html = chrome_error_html_str.as_bytes();

    assert!(
        is_chrome_error_page(chrome_error_html),
        "should detect Chrome error page by structural tail match"
    );
    assert!(
        !validate_empty(&Some(chrome_error_html.to_vec()), true),
        "Chrome error page should be treated as empty/invalid content"
    );

    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(chrome_error_html.to_vec()),
        ..Default::default()
    };
    let page = build("https://www.example.com", res);
    assert!(
        page.should_retry,
        "Chrome error page with 200 status should trigger retry"
    );
    assert_eq!(
        page.status_code,
        StatusCode::from_u16(599).unwrap(),
        "Chrome error page should be reclassified to 599"
    );
    assert!(
        !page.get_html().is_empty(),
        "Chrome error page content should be preserved for debugging"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_normal_page_not_detected_as_chrome_error() {
    let normal_html =
        b"<html><head><title>My Blog</title></head><body><p>Hello world</p></body></html>";
    assert!(!is_chrome_error_page(normal_html));
    assert!(validate_empty(&Some(normal_html.to_vec()), true));
}

// ---------------------------------------------------------------------------
// Chrome DNS-error reclassification (a.com / lema-gbr.de behaviour parity)
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_is_chrome_name_resolution_error_matches_both_forms() {
    // Accept both the CDP failure_text form and the JSON errorCode form.
    assert!(is_chrome_name_resolution_error(
        "net::ERR_NAME_NOT_RESOLVED"
    ));
    assert!(is_chrome_name_resolution_error("ERR_NAME_NOT_RESOLVED"));
    assert!(is_chrome_name_resolution_error(
        "net::ERR_NAME_RESOLUTION_FAILED"
    ));
    assert!(is_chrome_name_resolution_error(
        "ERR_NAME_RESOLUTION_FAILED"
    ));
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_is_chrome_name_resolution_error_rejects_transient_and_unrelated() {
    // Transient DNS issues remain retryable — they are NOT permanent failures.
    assert!(!is_chrome_name_resolution_error("net::ERR_DNS_TIMED_OUT"));
    assert!(!is_chrome_name_resolution_error(
        "net::ERR_DNS_SERVER_FAILED"
    ));
    assert!(!is_chrome_name_resolution_error(
        "net::ERR_DNS_MALFORMED_RESPONSE"
    ));
    // Unrelated net errors must not be misclassified as DNS.
    assert!(!is_chrome_name_resolution_error(
        "net::ERR_TUNNEL_CONNECTION_FAILED"
    ));
    assert!(!is_chrome_name_resolution_error("net::ERR_FAILED"));
    assert!(!is_chrome_name_resolution_error(
        "net::ERR_CONNECTION_REFUSED"
    ));
    assert!(!is_chrome_name_resolution_error(""));
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_extract_chrome_error_code_basic() {
    let padding = "x".repeat(1000);
    let html = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_NAME_NOT_RESOLVED\",\
         \"title\":\"a.com\"}};</script></html>"
    );
    assert_eq!(
        extract_chrome_error_code(html.as_bytes()),
        Some("ERR_NAME_NOT_RESOLVED")
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_extract_chrome_error_code_absent() {
    let html = b"<html><body><p>no error marker here</p></body></html>";
    assert_eq!(extract_chrome_error_code(html), None);
}

/// Rendered Chrome error page carrying ERR_NAME_NOT_RESOLVED must be
/// reclassified to 525 (DNS resolve error, permanent) instead of the
/// generic 599 — matching the behaviour for URLs whose DNS records do
/// not exist (e.g. `https://a.com`).
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_dns_reclassified_to_525() {
    let padding = "x".repeat(1000);
    let html_str = format!(
        "<html lang=\"en\" dir=\"ltr\">\n\
         <style>{padding}</style>\n\
         <div id=\"main-frame-error\" class=\"interstitial-wrapper\">\n\
         <h1><span>This site can\u{2019}t be reached</span></h1>\n\
         <div class=\"error-code\">ERR_NAME_NOT_RESOLVED</div>\n\
         </div>\n\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_NAME_NOT_RESOLVED\",\
         \"title\":\"a.com\"}};</script></html>"
    );
    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(html_str.into_bytes()),
        ..Default::default()
    };
    let page = build("https://a.com", res);
    assert_eq!(
        page.status_code, *DNS_RESOLVE_ERROR,
        "Chrome ERR_NAME_NOT_RESOLVED must reclassify to 525, not 599"
    );
    assert!(
        !page.should_retry,
        "DNS resolution failures must not trigger a retry"
    );
    assert!(
        !page.needs_retry(),
        "needs_retry() must be false for permanent DNS failures"
    );
}

/// Hickory resolver error strings (emitted when the `dns_cache` feature
/// wraps hickory for reqwest) must be recognised by the string-scan safety
/// net, so permanent DNS failures without a typed `io::Error(NotFound)`
/// source still map to 525 instead of being retried as transient.
#[test]
fn test_dns_error_ac_matches_hickory_strings() {
    assert!(
        DNS_ERROR_AC.is_match(
            "no record found for Query { name: Name(\"a.com.\"), query_type: A, query_class: IN }"
        ),
        "hickory NoRecordsFound Display must be caught by the safety-net AC scan"
    );
    assert!(
        DNS_ERROR_AC.is_match("dns resolution returned no addresses"),
        "DnsCacheResolver empty-addresses error must be caught"
    );
}

/// The pre-existing resolver strings must continue to match after extending
/// the automaton — regression guard for getaddrinfo / glibc / Node errors.
#[test]
fn test_dns_error_ac_matches_existing_resolver_strings() {
    assert!(DNS_ERROR_AC
        .is_match("dns error: failed to lookup address information: nodename nor servname"));
    assert!(
        DNS_ERROR_AC.is_match("error trying to connect: Name or service not known (os error -2)")
    );
    assert!(DNS_ERROR_AC.is_match("No address associated with hostname"));
    assert!(DNS_ERROR_AC.is_match("getaddrinfo ENOTFOUND example.invalid"));
}

/// Unrelated transport errors must NOT be misclassified as DNS — otherwise
/// legitimate transient failures (refused, reset, TLS) would stop retrying.
#[test]
fn test_dns_error_ac_rejects_unrelated_errors() {
    assert!(!DNS_ERROR_AC.is_match("connection refused"));
    assert!(!DNS_ERROR_AC.is_match("connection reset by peer"));
    assert!(!DNS_ERROR_AC.is_match("tls handshake failure"));
    assert!(!DNS_ERROR_AC.is_match("request timed out"));
    assert!(!DNS_ERROR_AC.is_match("broken pipe"));
    assert!(!DNS_ERROR_AC.is_match(""));
}

// ---------------------------------------------------------------------------
// is_chrome_permanent_failure — broader classifier for target-side failures
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_is_chrome_permanent_failure_accepts_all_target_side_codes() {
    for code in [
        "net::ERR_NAME_NOT_RESOLVED",
        "ERR_NAME_NOT_RESOLVED",
        "net::ERR_NAME_RESOLUTION_FAILED",
        "ERR_NAME_RESOLUTION_FAILED",
        "net::ERR_ADDRESS_UNREACHABLE",
        "ERR_ADDRESS_UNREACHABLE",
        "net::ERR_CONNECTION_REFUSED",
        "ERR_CONNECTION_REFUSED",
    ] {
        assert!(
            is_chrome_permanent_failure(code),
            "{code} must classify as a permanent target-side failure"
        );
    }
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_is_chrome_permanent_failure_rejects_transient_and_unrelated() {
    // Transient / retryable conditions stay eligible for retry.
    for code in [
        "net::ERR_DNS_TIMED_OUT",
        "net::ERR_DNS_SERVER_FAILED",
        "net::ERR_DNS_MALFORMED_RESPONSE",
        "net::ERR_TUNNEL_CONNECTION_FAILED",
        "net::ERR_PROXY_CONNECTION_FAILED",
        "net::ERR_CONNECTION_RESET",
        "net::ERR_CONNECTION_TIMED_OUT",
        "net::ERR_TIMED_OUT",
        "net::ERR_CERT_INVALID",
        "net::ERR_FAILED",
        "",
    ] {
        assert!(
            !is_chrome_permanent_failure(code),
            "{code} must remain retryable"
        );
    }
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_permanent_failure_status_maps_dns_to_525() {
    // NAME_* codes stay on 525 (DNS) for backward compatibility with any
    // downstream that pattern-matches specifically on 525.
    assert_eq!(
        chrome_permanent_failure_status("net::ERR_NAME_NOT_RESOLVED"),
        *DNS_RESOLVE_ERROR
    );
    assert_eq!(
        chrome_permanent_failure_status("ERR_NAME_RESOLUTION_FAILED"),
        *DNS_RESOLVE_ERROR
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_permanent_failure_status_maps_unreachable_to_526() {
    // ADDRESS_UNREACHABLE and CONNECTION_REFUSED get the distinct 526 code
    // so operators can distinguish "DNS dead" from "reachable-but-refused".
    assert_eq!(
        chrome_permanent_failure_status("net::ERR_ADDRESS_UNREACHABLE"),
        *ADDRESS_UNREACHABLE_ERROR
    );
    assert_eq!(
        chrome_permanent_failure_status("ERR_CONNECTION_REFUSED"),
        *ADDRESS_UNREACHABLE_ERROR
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_is_retryable_status_excludes_address_unreachable() {
    assert!(
        !is_retryable_status(*ADDRESS_UNREACHABLE_ERROR),
        "526 (ADDRESS_UNREACHABLE_ERROR) must not be retryable"
    );
    // The distinct-but-similar 525 stays non-retryable too (regression guard).
    assert!(
        !is_retryable_status(*DNS_RESOLVE_ERROR),
        "525 (DNS_RESOLVE_ERROR) must not be retryable"
    );
    // Generic 5xx codes that aren't our permanent markers remain retryable.
    assert!(is_retryable_status(StatusCode::from_u16(502).unwrap()));
    assert!(is_retryable_status(StatusCode::from_u16(503).unwrap()));
    assert!(is_retryable_status(StatusCode::from_u16(521).unwrap()));
}

/// `SSL_HANDSHAKE_ERROR_AC` must match the surfaces emitted by both reqwest
/// TLS backends and Chrome-wrapped error strings. Hits here flow through
/// `is_ssl_handshake_error` → 526 (`ADDRESS_UNREACHABLE_ERROR`) so the
/// page/website retry chain treats the destination as permanently
/// unreachable instead of cycling proxies for a TLS mismatch that no
/// connection rotation can fix.
#[test]
fn test_ssl_handshake_error_ac_matches_known_surfaces() {
    for s in [
        // rustls — full and short forms
        "received fatal alert: HandshakeFailure",
        "alert: HandshakeFailure",
        "alert: ProtocolVersion",
        // OpenSSL / native-tls
        "error:1408F10B:SSL routines:ssl3_get_record:wrong version number",
        "error:1408A0C1:SSL routines:ssl3_get_client_hello:no shared cipher",
        "tls error: unsupported protocol",
        // generic / wrappers
        "tls handshake error",
        "TLS handshake failed",
        "ssl handshake failed",
        "TLS handshake failure",
        // Chrome surface (ChromeMessage strings can leak into wrapped reqwest errors)
        "net::ERR_SSL_VERSION_OR_CIPHER_MISMATCH",
        "net::ERR_SSL_PROTOCOL_ERROR",
    ] {
        assert!(
            SSL_HANDSHAKE_ERROR_AC.is_match(s),
            "{s:?} must classify as SSL handshake failure"
        );
    }
}

/// Unrelated errors must NOT match the SSL detector — guards against the
/// pattern set widening to swallow transient connect/dns/timeout errors,
/// AND against URL-borne false positives. The tightened patterns now require
/// punctuation/verb tokens that don't survive URL encoding, so a URL like
/// `https://example.com/blog/tls-handshake-overview?topic=ProtocolVersion`
/// no longer matches.
#[test]
fn test_ssl_handshake_error_ac_rejects_unrelated() {
    for s in [
        // Unrelated connect/transport
        "connection reset by peer",
        "broken pipe",
        "operation timed out",
        // DNS surfaces (handled separately by DNS_ERROR_AC / 525)
        "dns error: ENOTFOUND",
        "failed to lookup address",
        // Plain HTTP/protocol noise
        "invalid HTTP response",
        "decode error",
        "",
        // URL-borne false-positive guards — tightened patterns won't trip:
        "error sending request for url (https://example.com/blog/tls-handshake-overview)",
        "error sending request for url (https://example.com/blog/ssl-handshake-explained)",
        "error sending request for url (https://example.com/?topic=ProtocolVersion)",
        "error sending request for url (https://example.com/HandshakeFailure-explained)",
    ] {
        assert!(
            !SSL_HANDSHAKE_ERROR_AC.is_match(s),
            "{s:?} must NOT classify as SSL handshake failure"
        );
    }
}

/// SSL handshake errors must surface as 526 (`ADDRESS_UNREACHABLE_ERROR`),
/// which `is_retryable_status` excludes — so neither the in-page retry loop
/// nor the website-level retry loop will hammer an origin whose TLS stack
/// is fundamentally incompatible with the client.
#[test]
fn test_is_retryable_status_excludes_ssl_handshake_bucket() {
    // 526 is the bucket SSL handshake failures land in.
    assert!(
        !is_retryable_status(*ADDRESS_UNREACHABLE_ERROR),
        "526 must remain non-retryable for SSL handshake failures"
    );
}

/// 501, 505, 511 are 5xx codes whose semantics are deterministic — the
/// server is announcing a "won't do" condition that no retry can change.
/// They must be excluded from `is_retryable_status` so the website retry
/// loop skips them, and from `build()`'s `should_retry_status` so the page
/// itself does not flag itself for retry.
#[test]
fn test_is_retryable_status_excludes_permanent_5xx() {
    for code in [
        StatusCode::NOT_IMPLEMENTED,                 // 501
        StatusCode::HTTP_VERSION_NOT_SUPPORTED,      // 505
        StatusCode::NETWORK_AUTHENTICATION_REQUIRED, // 511
    ] {
        assert!(
            !is_retryable_status(code),
            "{code} must be permanent (server-declared won't-do)"
        );
    }
    // Other 5xx still retryable — regression guard.
    for code in [
        StatusCode::INTERNAL_SERVER_ERROR, // 500
        StatusCode::BAD_GATEWAY,           // 502
        StatusCode::SERVICE_UNAVAILABLE,   // 503
        StatusCode::GATEWAY_TIMEOUT,       // 504
    ] {
        assert!(is_retryable_status(code), "{code} must remain retryable");
    }
}

/// End-to-end guard for the new permanent 5xx codes: a `PageResponse` with
/// 501 / 505 / 511 must yield a `Page` whose `should_retry=false` and
/// `needs_retry()=false`. Mirrors the SSL end-to-end test.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_permanent_5xx_pages_are_not_retryable_end_to_end() {
    for code in [
        StatusCode::NOT_IMPLEMENTED,
        StatusCode::HTTP_VERSION_NOT_SUPPORTED,
        StatusCode::NETWORK_AUTHENTICATION_REQUIRED,
    ] {
        let res = PageResponse {
            status_code: code,
            content: None,
            ..Default::default()
        };
        let page = build("https://example.invalid", res);
        assert_eq!(page.status_code, code);
        assert!(
            !page.should_retry,
            "Page::should_retry must be false for {code}"
        );
        assert!(
            !page.needs_retry(),
            "Page::needs_retry() must be false for {code}"
        );
    }
}

/// New Chrome `net::ERR_*` codes (h2 inadequate security + URL-malformed
/// family) must classify as permanent and route to the right status bucket.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_is_chrome_permanent_failure_extended_codes() {
    for code in [
        // h2 inadequate security — bucketed with SSL family at 526
        "ERR_HTTP2_INADEQUATE_TRANSPORT_SECURITY",
        "net::ERR_HTTP2_INADEQUATE_TRANSPORT_SECURITY",
        // URL-malformed family — bucketed at 400
        "ERR_INVALID_URL",
        "ERR_UNSAFE_PORT",
        "ERR_DISALLOWED_URL_SCHEME",
        "ERR_UNKNOWN_URL_SCHEME",
        "net::ERR_INVALID_URL",
    ] {
        assert!(
            is_chrome_permanent_failure(code),
            "{code} must classify as a permanent Chrome failure"
        );
    }
}

/// `chrome_permanent_failure_status` must route the new codes correctly:
/// h2 inadequate security → 526 (origin unreachable bucket), URL-malformed
/// family → 400 (BAD_REQUEST). Both are excluded from `is_retryable_status`.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_permanent_failure_status_routes_extended_codes() {
    assert_eq!(
        chrome_permanent_failure_status("net::ERR_HTTP2_INADEQUATE_TRANSPORT_SECURITY"),
        *ADDRESS_UNREACHABLE_ERROR
    );
    for code in [
        "ERR_INVALID_URL",
        "ERR_UNSAFE_PORT",
        "ERR_DISALLOWED_URL_SCHEME",
        "ERR_UNKNOWN_URL_SCHEME",
    ] {
        assert_eq!(
            chrome_permanent_failure_status(code),
            StatusCode::BAD_REQUEST,
            "{code} must route to 400"
        );
        assert!(
            !is_retryable_status(chrome_permanent_failure_status(code)),
            "{code} status must not be retryable"
        );
    }
}

/// End-to-end guard: a `PageResponse` whose Chrome error page declares an
/// `errorCode` from the new permanent set must reclassify and refuse retry.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_extended_codes_reclassified() {
    let cases: &[(&str, StatusCode)] = &[
        (
            "ERR_HTTP2_INADEQUATE_TRANSPORT_SECURITY",
            *ADDRESS_UNREACHABLE_ERROR,
        ),
        ("ERR_INVALID_URL", StatusCode::BAD_REQUEST),
        ("ERR_UNSAFE_PORT", StatusCode::BAD_REQUEST),
    ];
    let padding = "x".repeat(1000);
    for (code, expected) in cases {
        let html_str = format!(
            "<html><style>{padding}</style>\
             <script>var loadTimeDataRaw = {{\"errorCode\":\"{code}\",\
             \"title\":\"example.invalid\"}};</script></html>"
        );
        let res = PageResponse {
            status_code: StatusCode::OK,
            content: Some(html_str.into_bytes()),
            ..Default::default()
        };
        let page = build("https://example.invalid", res);
        assert_eq!(
            page.status_code, *expected,
            "Chrome {code} must reclassify to {expected}"
        );
        assert!(
            !page.should_retry,
            "{code} must not flag the page for retry"
        );
        assert!(
            !page.needs_retry(),
            "{code} must not trigger website-level retry"
        );
    }
}

/// End-to-end guard for the SSL classification: a `PageResponse` produced
/// by the Chrome fallback (status 526, empty body) must yield a `Page` with
/// `should_retry=false` and `needs_retry()=false`. This is what stops the
/// website-level retry loop and prevents the page itself from looping —
/// matching the user's contract: "the page should not have should_retry on
/// it and the website should not retry it".
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_ssl_handshake_page_is_not_retryable_end_to_end() {
    let res = PageResponse {
        status_code: *ADDRESS_UNREACHABLE_ERROR,
        content: None,
        ..Default::default()
    };
    let page = build("https://www.example.invalid/legal/impressum", res);
    assert_eq!(
        page.status_code, *ADDRESS_UNREACHABLE_ERROR,
        "SSL handshake page must keep its 526 classification through build()"
    );
    assert!(
        !page.should_retry,
        "Page::should_retry must be false for SSL handshake failures"
    );
    assert!(
        !page.needs_retry(),
        "Page::needs_retry() must be false so the website retry loop skips"
    );
}

/// `is_chrome_name_resolution_error` is a public API and must keep its
/// narrower (NAME-only) semantics — callers depending on the old behaviour
/// should not see newly-matched codes leak through.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_is_chrome_name_resolution_error_backward_compat() {
    assert!(is_chrome_name_resolution_error("ERR_NAME_NOT_RESOLVED"));
    assert!(is_chrome_name_resolution_error(
        "ERR_NAME_RESOLUTION_FAILED"
    ));
    // The broader permanent-failure codes must NOT flow through the older
    // narrow helper — that's what `is_chrome_permanent_failure` is for.
    assert!(!is_chrome_name_resolution_error("ERR_ADDRESS_UNREACHABLE"));
    assert!(!is_chrome_name_resolution_error("ERR_CONNECTION_REFUSED"));
}

/// Rendered Chrome error page carrying ERR_ADDRESS_UNREACHABLE must be
/// reclassified to 526 (address unreachable, permanent) instead of the
/// generic 599. This is the SOCKS-0x04-through-proxy case the fix targets.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_address_unreachable_reclassified_to_526() {
    let padding = "x".repeat(1000);
    let html_str = format!(
        "<html lang=\"en\" dir=\"ltr\">\n\
         <style>{padding}</style>\n\
         <div class=\"error-code\">ERR_ADDRESS_UNREACHABLE</div>\n\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_ADDRESS_UNREACHABLE\",\
         \"title\":\"example.invalid\"}};</script></html>"
    );
    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(html_str.into_bytes()),
        ..Default::default()
    };
    let page = build("https://example.invalid", res);
    assert_eq!(
        page.status_code, *ADDRESS_UNREACHABLE_ERROR,
        "Chrome ERR_ADDRESS_UNREACHABLE must reclassify to 526, not 599"
    );
    assert!(
        !page.should_retry,
        "address-unreachable failures must not trigger a retry"
    );
    assert!(
        !page.needs_retry(),
        "needs_retry() must be false for permanent address-unreachable failures"
    );
}

/// Rendered Chrome error page carrying ERR_CONNECTION_REFUSED must be
/// reclassified to 526 so the retry chain doesn't cycle through every
/// configured proxy for a target that has actively refused the connection.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_connection_refused_reclassified_to_526() {
    let padding = "x".repeat(1000);
    let html_str = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_CONNECTION_REFUSED\",\
         \"title\":\"example.invalid\"}};</script></html>"
    );
    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(html_str.into_bytes()),
        ..Default::default()
    };
    let page = build("https://example.invalid", res);
    assert_eq!(
        page.status_code, *ADDRESS_UNREACHABLE_ERROR,
        "Chrome ERR_CONNECTION_REFUSED must reclassify to 526"
    );
    assert!(
        !page.should_retry,
        "connection-refused failures through proxy chain must not retry"
    );
}

/// Non-DNS Chrome error pages keep the existing 599 (retryable) behaviour so
/// proxy / tunnel / TLS failures still get rotated through the retry path.
#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_non_dns_stays_599() {
    let padding = "x".repeat(1000);
    let html_str = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_TUNNEL_CONNECTION_FAILED\",\
         \"title\":\"example.com\"}};</script></html>"
    );
    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(html_str.into_bytes()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    assert_eq!(
        page.status_code,
        StatusCode::from_u16(599).unwrap(),
        "non-DNS Chrome error pages must still map to 599"
    );
    assert!(
        page.should_retry,
        "599 errors remain retryable for proxy/tunnel rotation"
    );
}

// ---------------------------------------------------------------------------
// is_retryable_status — exhaustive coverage
// ---------------------------------------------------------------------------

#[test]
fn test_retryable_status_server_errors() {
    // 5xx codes are retryable EXCEPT the permanent set: 525 (DNS), 526
    // (origin-unreachable / SSL-handshake), 501 (Not Implemented), 505 (HTTP
    // Version Not Supported), 511 (Network Auth Required). Server explicitly
    // declared a "won't do" condition for those — retrying just adds load.
    for code in [500, 502, 503, 504, 521, 522, 523, 524, 598, 599] {
        let status = StatusCode::from_u16(code).unwrap();
        assert!(is_retryable_status(status), "{code} should be retryable");
    }
    for code in [501, 505, 511, 525, 526] {
        let status = StatusCode::from_u16(code).unwrap();
        assert!(
            !is_retryable_status(status),
            "{code} must NOT be retryable (permanent declaration)"
        );
    }
}

#[test]
fn test_retryable_status_rate_limit_and_timeout() {
    assert!(
        is_retryable_status(StatusCode::TOO_MANY_REQUESTS),
        "429 retryable"
    );
    assert!(
        is_retryable_status(StatusCode::REQUEST_TIMEOUT),
        "408 retryable"
    );
}

#[test]
fn test_non_retryable_status_dns_error() {
    let dns = StatusCode::from_u16(525).unwrap();
    assert!(!is_retryable_status(dns), "525 DNS must never be retried");
}

#[test]
fn test_non_retryable_client_errors() {
    for code in [400, 401, 403, 404, 405, 409, 410, 422, 451] {
        let status = StatusCode::from_u16(code).unwrap();
        assert!(
            !is_retryable_status(status),
            "{code} should NOT be retryable"
        );
    }
}

#[test]
fn test_non_retryable_success_codes() {
    for code in [200, 201, 204, 301, 302, 304] {
        let status = StatusCode::from_u16(code).unwrap();
        assert!(
            !is_retryable_status(status),
            "{code} should NOT be retryable"
        );
    }
}

// ---------------------------------------------------------------------------
// needs_retry — combinatorial coverage
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_should_retry_flag_alone() {
    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(b"<html><body>ok</body></html>".to_vec()),
        ..Default::default()
    };
    let mut page = build("https://example.com", res);
    assert!(!page.needs_retry(), "clean 200 page should not need retry");

    page.should_retry = true;
    assert!(page.needs_retry(), "should_retry flag forces retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_content_truncated_alone() {
    let res = PageResponse {
        status_code: StatusCode::OK,
        content: Some(b"<html><body>ok</body></html>".to_vec()),
        ..Default::default()
    };
    let mut page = build("https://example.com", res);
    page.content_truncated = true;
    assert!(page.needs_retry(), "truncated content forces retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_retryable_status_alone() {
    let res = PageResponse {
        status_code: StatusCode::BAD_GATEWAY,
        content: Some(Default::default()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    // Even if should_retry wasn't explicitly set by build(), needs_retry
    // catches it via is_retryable_status.
    assert!(page.needs_retry(), "502 status triggers needs_retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_dns_error_not_retried() {
    let res = PageResponse {
        status_code: StatusCode::from_u16(525).unwrap(),
        content: None,
        ..Default::default()
    };
    let page = build("https://nonexistent.invalid", res);
    // DNS errors (525) are explicitly excluded from is_retryable_status AND
    // should not set should_retry either.
    assert!(!page.needs_retry(), "DNS 525 must never trigger retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_client_error_no_flags() {
    let res = PageResponse {
        status_code: StatusCode::NOT_FOUND,
        content: Some(b"<html>not found</html>".to_vec()),
        ..Default::default()
    };
    let page = build("https://example.com/missing", res);
    assert!(!page.needs_retry(), "404 should not need retry");
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_multiple_flags_combined() {
    let res = PageResponse {
        status_code: StatusCode::SERVICE_UNAVAILABLE,
        content: Some(Default::default()),
        ..Default::default()
    };
    let mut page = build("https://example.com", res);
    page.content_truncated = true;
    // Both retryable status AND truncated — still just needs_retry = true
    assert!(
        page.needs_retry(),
        "multiple retry signals still returns true"
    );
}

// ---------------------------------------------------------------------------
// is_chrome_error_page — edge cases
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_under_500_bytes() {
    let short = b"<script>var loadTimeDataRaw = {\"errorCode\":\"ERR_FAIL\"};</script></html>";
    assert!(
        !is_chrome_error_page(short),
        "content < 500 bytes should be rejected"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_missing_tail() {
    let padding = "x".repeat(1000);
    let html = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_FAIL\"}};</script></body></html>"
    );
    assert!(
        !is_chrome_error_page(html.as_bytes()),
        "wrong tail (has </body>) should not match"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_missing_error_code_needle() {
    let padding = "x".repeat(1000);
    let html = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"someKey\":\"value\"}};</script></html>"
    );
    assert!(
        !is_chrome_error_page(html.as_bytes()),
        "missing errorCode needle should not match"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_trailing_whitespace() {
    let padding = "x".repeat(1000);
    let html = format!(
        "<html><style>{padding}</style>\
         <script>var loadTimeDataRaw = {{\"errorCode\":\"ERR_TUNNEL_CONNECTION_FAILED\"}};</script></html>\n\r\n  "
    );
    assert!(
        is_chrome_error_page(html.as_bytes()),
        "trailing whitespace should be trimmed"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_chrome_error_page_needle_outside_4kb_window() {
    // Put the errorCode at the start, then pad >4KB before the tail.
    // The needle should NOT be found because we only scan the last 4KB.
    let error_part = r#"<script>var loadTimeDataRaw = {"errorCode":"ERR_FAIL"};</script>"#;
    let padding = "x".repeat(5000); // >4KB of padding after needle
    let html = format!(
        "<html>{error_part}<style>{padding}</style>\
         <script>var more = {{}};</script></html>"
    );
    assert!(
        !is_chrome_error_page(html.as_bytes()),
        "needle outside last 4KB window should not match"
    );
}

// ---------------------------------------------------------------------------
// get_timeout — status-specific delay values
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_get_timeout_rate_limit() {
    let res = PageResponse {
        status_code: StatusCode::TOO_MANY_REQUESTS,
        content: None,
        ..Default::default()
    };
    let page = build("https://example.com", res);
    let timeout = page.get_timeout();
    assert_eq!(
        timeout,
        Some(std::time::Duration::from_millis(2_500)),
        "429 → 2500ms"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_get_timeout_gateway_timeout() {
    let res = PageResponse {
        status_code: StatusCode::GATEWAY_TIMEOUT,
        content: None,
        ..Default::default()
    };
    let page = build("https://example.com", res);
    let timeout = page.get_timeout();
    assert_eq!(
        timeout,
        Some(std::time::Duration::from_millis(1_500)),
        "504 → 1500ms"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_get_timeout_proxy_errors() {
    for code in [598u16, 599] {
        let res = PageResponse {
            status_code: StatusCode::from_u16(code).unwrap(),
            content: None,
            ..Default::default()
        };
        let page = build("https://example.com", res);
        let timeout = page.get_timeout();
        assert_eq!(
            timeout,
            Some(std::time::Duration::from_millis(500)),
            "{code} → 500ms"
        );
    }
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_get_timeout_normal_status_none() {
    for code in [200u16, 301, 404, 500, 502, 503] {
        let res = PageResponse {
            status_code: StatusCode::from_u16(code).unwrap(),
            content: Some(b"<html></html>".to_vec()),
            ..Default::default()
        };
        let page = build("https://example.com", res);
        let timeout = page.get_timeout();
        assert_eq!(timeout, None, "{code} should have no special timeout");
    }
}

// ---------------------------------------------------------------------------
// validate_empty — edge cases
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_validate_empty_none_content() {
    assert!(!validate_empty(&None, true), "None content is empty");
    assert!(
        !validate_empty(&None, false),
        "None content is empty regardless of success"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_validate_empty_zero_length() {
    assert!(!validate_empty(&Some(vec![]), true), "empty vec is empty");
    assert!(
        !validate_empty(&Some(vec![]), false),
        "empty vec is empty regardless of success"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_validate_empty_html_shell() {
    let shell = b"<html><head></head><body></body></html>".to_vec();
    assert!(
        !validate_empty(&Some(shell), true),
        "empty HTML shell should be rejected"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_validate_empty_valid_content() {
    let valid = b"<html><head><title>Test</title></head><body><p>Hello</p></body></html>".to_vec();
    assert!(validate_empty(&Some(valid), true), "valid HTML should pass");
}

// ---------------------------------------------------------------------------
// 401 proxy-conditional retry
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_401_not_retried_without_proxy() {
    let res = PageResponse {
        status_code: StatusCode::UNAUTHORIZED,
        content: Some(b"unauthorized".to_vec()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    assert!(
        !page.needs_retry(),
        "401 without proxy_configured should NOT trigger retry"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_401_retried_with_proxy() {
    let res = PageResponse {
        status_code: StatusCode::UNAUTHORIZED,
        content: Some(b"unauthorized".to_vec()),
        ..Default::default()
    };
    let mut page = build("https://example.com", res);
    page.proxy_configured = true;
    assert!(
        page.needs_retry(),
        "401 with proxy_configured should trigger retry"
    );
}

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_401_proxy_flag_does_not_affect_other_client_errors() {
    // 404 (with content) should NOT be retried even with proxy
    let res = PageResponse {
        status_code: StatusCode::NOT_FOUND,
        content: Some(b"<html><body>Not Found</body></html>".to_vec()),
        ..Default::default()
    };
    let mut page = build("https://example.com/missing", res);
    page.proxy_configured = true;
    assert!(
        !page.needs_retry(),
        "404 should NOT be retried even with proxy"
    );
}

// ---------------------------------------------------------------------------
// needs_retry integration: proxy + other flags
// ---------------------------------------------------------------------------

#[cfg(not(feature = "decentralized"))]
#[test]
fn test_needs_retry_server_error_regardless_of_proxy() {
    let res = PageResponse {
        status_code: StatusCode::INTERNAL_SERVER_ERROR,
        content: Some(Default::default()),
        ..Default::default()
    };
    let page = build("https://example.com", res);
    // 500 is retryable regardless of proxy config
    assert!(page.needs_retry(), "500 retried without proxy");
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

impl crate::traits::PageData for Page {
    #[inline]
    fn url(&self) -> &str {
        self.get_url()
    }

    #[inline]
    fn url_final(&self) -> &str {
        match self.final_redirect_destination.as_deref() {
            Some(u) => u,
            _ => &self.url,
        }
    }

    #[inline]
    fn bytes(&self) -> Option<&[u8]> {
        self.get_bytes()
    }

    #[inline]
    fn html(&self) -> String {
        self.get_html()
    }

    #[inline]
    fn html_bytes_u8(&self) -> &[u8] {
        self.get_html_bytes_u8()
    }

    #[inline]
    fn status_code(&self) -> StatusCode {
        self.status_code
    }

    #[inline]
    fn headers(&self) -> Option<&reqwest::header::HeaderMap> {
        self.headers.as_ref()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

#[cfg(feature = "time")]
impl crate::traits::PageTimingExt for Page {
    #[inline]
    fn duration_elapsed(&self) -> tokio::time::Duration {
        self.get_duration_elapsed()
    }
}

#[cfg(feature = "chrome")]
impl crate::traits::PageChromeExt for Page {
    #[inline]
    fn chrome_page(&self) -> Option<&chromiumoxide::Page> {
        self.get_chrome_page()
    }

    #[inline]
    fn screenshot_bytes(&self) -> Option<&[u8]> {
        self.screenshot_bytes.as_deref()
    }
}

// ============================================================================
// Empty-success reclassification tests
// ============================================================================

#[cfg(all(test, not(feature = "decentralized")))]
mod empty_success_tests {
    use super::*;
    use crate::utils::PageResponse;

    fn pr(status: u16, content: Option<Vec<u8>>) -> PageResponse {
        PageResponse {
            status_code: StatusCode::from_u16(status).unwrap(),
            content,
            ..Default::default()
        }
    }

    /// 200 OK with `content: None` (e.g. proxy edge-blocked → spider's
    /// chrome-fallback HTTP path constructs `PageResponse { content: None,
    /// status_code: 200 }`) must surface as failure to consumers.
    #[test]
    fn empty_success_none_content_reclassified_to_504() {
        let page = build("https://example.com", pr(200, None));
        assert_eq!(page.status_code.as_u16(), 504);
        assert!(page.should_retry, "must flag for retry strategy");
    }

    /// 200 OK with an empty Vec body — same failure mode, surfaced via
    /// the chrome stream-success path that wrote zero bytes.
    #[test]
    fn empty_success_empty_vec_reclassified_to_504() {
        let page = build("https://example.com", pr(200, Some(Vec::new())));
        assert_eq!(page.status_code.as_u16(), 504);
        assert!(page.should_retry);
    }

    /// 200 OK with a Chrome empty-shell body — production sees this when
    /// CDP gives back the bare `<html><head></head><body></body></html>`
    /// template instead of real content.
    #[test]
    fn empty_success_shell_html_reclassified_to_504() {
        let page = build(
            "https://example.com",
            pr(
                200,
                Some(b"<html><head></head><body></body></html>".to_vec()),
            ),
        );
        assert_eq!(page.status_code.as_u16(), 504);
        assert!(page.should_retry);
    }

    /// Real 200 with real content must not be reclassified — guard
    /// against false-positive regressions on the happy path.
    #[test]
    fn real_success_preserves_status_200() {
        let html = b"<html><head><title>x</title></head><body><h1>hi</h1></body></html>";
        let page = build("https://example.com", pr(200, Some(html.to_vec())));
        assert_eq!(page.status_code.as_u16(), 200);
    }

    /// 404 with empty body must keep its 404 — `validate_empty(_, true)`
    /// already treats 404 as a permitted "success" (NotFound carries
    /// useful semantics), and we should not promote it to 504.
    #[test]
    fn not_found_empty_body_keeps_404() {
        let page = build("https://example.com", pr(404, None));
        assert_eq!(page.status_code.as_u16(), 404);
    }

    /// Truncated bodies preserve the original 200 — a partial body is
    /// real data the caller may want to inspect, not a silent failure.
    #[test]
    fn truncated_success_preserves_status() {
        let mut res = pr(200, Some(b"<html><body><p>partial".to_vec()));
        res.content_truncated = true;
        let page = build("https://example.com", res);
        assert_eq!(page.status_code.as_u16(), 200);
    }

    /// Pre-existing chrome-error reclassification (599 / 525 / 526) must
    /// still take precedence; the new empty-success branch is gated on
    /// `!chrome_error` to avoid double-rewriting.
    #[test]
    fn chrome_error_page_reclassified_to_599_not_504() {
        // Minimal chrome error fingerprint — content ends with the
        // chrome-error tail `};</script></html>` (no `</body>`).
        let body = b"<html><head></head><body><div id=\"main-frame-error\"></div></body><script>loadTimeDataRaw = {\"errorCode\":\"net::ERR_GENERIC\"};</script></html>";
        let page = build("https://example.com", pr(200, Some(body.to_vec())));
        // 599 is the spider-internal chrome error code. We just want to
        // confirm it is *not* 504 — the chrome_error branch handled it.
        assert_ne!(page.status_code.as_u16(), 504);
    }

    /// 5xx server errors retain their original status and should_retry=true.
    #[test]
    fn server_error_preserves_status() {
        let page = build("https://example.com", pr(503, None));
        assert_eq!(page.status_code.as_u16(), 503);
        assert!(page.should_retry);
    }

    /// Under the `balance` feature, a real page with bytes spooled to
    /// disk arrives with `res.content = None` while `res.content_spool`
    /// holds the on-disk handle. `validate_empty` only inspects
    /// `res.content`, so without the spool guard we would mis-classify
    /// the page as empty and downgrade its status to 504. Verify the
    /// guard preserves 200 OK on the spooled path.
    #[cfg(feature = "balance")]
    #[test]
    fn balance_spooled_content_preserves_status_200() {
        use crate::utils::html_spool::{SpoolVitals, SpooledContent};
        use std::path::PathBuf;

        let mut response = pr(200, None);
        response.content_spool = Some(SpooledContent {
            path: PathBuf::from("/tmp/spider-test-spooled-page.html"),
            vitals: SpoolVitals {
                byte_len: 70_000,
                is_valid_utf8: true,
                is_xml: false,
                binary_file: false,
            },
            ..Default::default()
        });
        let page = build("https://example.com", response);
        assert_eq!(
            page.status_code.as_u16(),
            200,
            "spooled content must keep its 200 status — bytes are on disk, not empty"
        );
    }

    /// `PageResponse::content_size` mirrors `Page::size`: spool vitals
    /// take precedence over the in-memory buffer length, and missing /
    /// empty content yields 0. Critical perf invariant — the spool file
    /// is never re-read; we copy the writer's inline counter.
    #[test]
    fn content_size_empty_response_returns_zero() {
        assert_eq!(pr(200, None).content_size(), 0);
    }

    #[test]
    fn content_size_in_memory_returns_buffer_len() {
        let body = b"<html><body>hello</body></html>".to_vec();
        let expected = body.len();
        assert_eq!(pr(200, Some(body)).content_size(), expected);
    }

    #[cfg(feature = "balance")]
    #[test]
    fn content_size_spool_returns_vitals_byte_len() {
        use crate::utils::html_spool::{SpoolVitals, SpooledContent};
        use std::path::PathBuf;
        let mut response = pr(200, None);
        response.content_spool = Some(SpooledContent {
            path: PathBuf::from("/tmp/spider-test-spool-size.html"),
            vitals: SpoolVitals {
                byte_len: 4_096,
                is_valid_utf8: true,
                is_xml: false,
                binary_file: false,
            },
            ..Default::default()
        });
        assert_eq!(response.content_size(), 4_096);
    }

    /// `has_content_bytes` is the cheaper presence check used by the
    /// empty-success guard. Returns true for spooled bytes (under
    /// balance) and for any non-empty in-memory content.
    #[test]
    fn has_content_bytes_empty_response_returns_false() {
        assert!(!pr(200, None).has_content_bytes());
        assert!(!pr(200, Some(Vec::new())).has_content_bytes());
    }

    #[test]
    fn has_content_bytes_in_memory_returns_true() {
        assert!(pr(200, Some(b"x".to_vec())).has_content_bytes());
    }

    #[cfg(feature = "balance")]
    #[test]
    fn has_content_bytes_spool_returns_true_even_with_empty_content() {
        use crate::utils::html_spool::{SpoolVitals, SpooledContent};
        use std::path::PathBuf;
        let mut response = pr(200, None);
        response.content_spool = Some(SpooledContent {
            path: PathBuf::from("/tmp/spider-test-spool-presence.html"),
            vitals: SpoolVitals {
                byte_len: 100,
                is_valid_utf8: true,
                is_xml: false,
                binary_file: false,
            },
            ..Default::default()
        });
        assert!(response.has_content_bytes());
    }

    /// Spooled pages on a 200 success path must NOT inherit
    /// `should_retry=true` from `should_retry_empty_success`. Before the
    /// metadata short-circuit `validate_empty(&res.content=None, _)` flagged
    /// every spooled page as "empty" → spurious retry flag on real
    /// content. Locks in the fix.
    #[cfg(feature = "balance")]
    #[test]
    fn balance_spooled_content_does_not_set_should_retry() {
        use crate::utils::html_spool::{SpoolVitals, SpooledContent};
        use std::path::PathBuf;
        let mut response = pr(200, None);
        response.content_spool = Some(SpooledContent {
            path: PathBuf::from("/tmp/spider-test-spool-no-retry.html"),
            vitals: SpoolVitals {
                byte_len: 50_000,
                is_valid_utf8: true,
                is_xml: false,
                binary_file: false,
            },
            ..Default::default()
        });
        let page = build("https://example.com", response);
        assert!(
            !page.should_retry,
            "spooled real content on 2xx must not be flagged for retry"
        );
    }

    /// Spool path surfaces the disk-bound byte count via `content_byte_len`
    /// (a balance-feature field separate from `bytes_transferred`, which is
    /// the chrome network-IO counter). The byte count is populated from
    /// the precomputed `vitals.byte_len` — the spool file is *never*
    /// re-read; we copy the counter the writer set inline with the disk
    /// flush.
    #[cfg(feature = "balance")]
    #[test]
    fn balance_spool_populates_content_byte_len_from_vitals() {
        use crate::utils::html_spool::{SpoolVitals, SpooledContent};
        use std::path::PathBuf;

        let mut response = pr(200, None);
        response.content_spool = Some(SpooledContent {
            path: PathBuf::from("/tmp/spider-test-spooled-page-vitals.html"),
            vitals: SpoolVitals {
                byte_len: 12_345,
                is_valid_utf8: true,
                is_xml: false,
                binary_file: false,
            },
            ..Default::default()
        });
        let page = build("https://example.com", response);
        assert_eq!(
            page.content_byte_len, 12_345,
            "content_byte_len must come from spool vitals — never re-read disk"
        );
    }
}
