//! Media-asset URL classification.
//!
//! A small, allocation-free helper for telling whether a URL points at a
//! "media asset" (image, video, audio, font, document, archive). Backed
//! by a compile-time `phf::Set<&'static str>` so the lookup is a single
//! perfect-hash probe with no runtime initialization.
//!
//! This is a pure technical classifier — it carries no policy, no cost
//! model, no business semantics. Use it from your
//! [`ProxyStrategy`](crate::proxy_strategy::ProxyStrategy) impl when
//! deciding whether to route a request through a kind-specific proxy
//! (typically [`ProxyKind::MediaAsset`](crate::proxy_strategy::ProxyKind::MediaAsset)).
//!
//! # Examples
//!
//! ```
//! use spider::utils::media_asset::{is_media_asset_url, is_media_asset_path};
//!
//! assert!(is_media_asset_url("https://example.com/foo.JPG"));
//! assert!(is_media_asset_url("https://example.com/dir/clip.mp4?token=abc"));
//! assert!(is_media_asset_path("/font.woff2"));
//!
//! assert!(!is_media_asset_url("https://example.com/index.html"));
//! assert!(!is_media_asset_url("not-a-url"));
//! ```

/// Compile-time set of media-asset extensions, lowercased. The
/// [`is_media_asset_path`] / [`is_media_asset_url`] helpers lowercase
/// the extracted extension before consulting this set so callers can
/// pass mixed-case URLs without doing it themselves.
///
/// Coverage is the union of the common asset families: raster + vector
/// images, web video and audio formats, web font formats, office /
/// reader documents, and archive / installer payloads. Adding a new
/// extension is a one-line edit; the only correctness requirement is
/// that the entry be lowercase ASCII.
static MEDIA_EXTS: phf::Set<&'static str> = phf::phf_set! {
    // images
    "jpg", "jpeg", "png", "gif", "svg", "webp",
    "bmp", "tiff", "tif", "heic", "heif", "ico", "apng", "avif",
    // video
    "mp4", "avi", "mov", "wmv", "flv",
    "mkv", "webm", "m4v",
    "ogv", "ogx", "mpeg", "ts", "3gp", "3g2",
    // audio
    "mp3", "wav", "ogg",
    "aac", "flac", "m4a", "aiff",
    "cda", "mid", "midi", "oga", "opus", "weba",
    // fonts
    "woff", "woff2", "ttf", "otf", "eot",
    // legacy plugin payloads
    "swf", "xap",
    // documents / data
    "pdf", "eps", "yaml", "yml", "rtf", "txt",
    "doc", "docx", "csv", "epub", "gz",
    "ics", "md", "webmanifest",
    "abw", "azw", "odt", "ods", "odp", "ppt", "pptx", "xls", "xlsx", "vsd",
    // archives / installers
    "arc", "bin", "bz", "bz2", "jar", "mpkg", "rar", "tar", "zip", "7z",
};

/// Is the given **path** a media-asset path by extension?
///
/// `path` should be just the path component of a URL (no query / fragment),
/// e.g. `/dir/file.png`. Pass a full URL to [`is_media_asset_url`]
/// instead, which strips query / fragment for you.
///
/// Allocation: zero on the lowercase fast path. When the extension
/// contains any uppercase byte, one transient allocation is made for
/// the lowercased copy so the lookup can hit the same compile-time set.
#[inline]
pub fn is_media_asset_path(path: &str) -> bool {
    let ext = match path.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => ext,
        _ => return false,
    };
    if ext.bytes().any(|b| b.is_ascii_uppercase()) {
        let lower = ext.to_ascii_lowercase();
        MEDIA_EXTS.contains(lower.as_str())
    } else {
        MEDIA_EXTS.contains(ext)
    }
}

/// Is the given **URL** a media-asset URL by extension?
///
/// Strips the query and fragment internally, then defers to
/// [`is_media_asset_path`]. Returns `false` for unparseable URLs and
/// for URLs whose path has no extension (e.g. `https://example.com/`).
///
/// Allocation: zero when the URL parses, has no query / fragment after
/// the extension, and the extension is already lowercase. Otherwise at
/// most one transient lowercase copy.
#[inline]
pub fn is_media_asset_url(url: &str) -> bool {
    // Find the path segment without paying for full URL parsing — we
    // only need the part between the host and the first `?` or `#`.
    // This keeps the hot path allocation-free and avoids pulling
    // `url::Url` into callers that already know the URL is well-formed.
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let path_and_rest = match after_scheme.find('/') {
        Some(i) => &after_scheme[i..],
        // No path component at all (e.g. "https://example.com").
        None => return false,
    };
    let path = path_and_rest
        .split_once(['?', '#'])
        .map(|(p, _)| p)
        .unwrap_or(path_and_rest);
    is_media_asset_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_common_media_extensions() {
        for url in [
            "https://example.com/foo.jpg",
            "https://example.com/foo.JPG",
            "http://cdn.example.com/dir/sub/clip.MP4",
            "https://example.com/file.pdf?token=abc",
            "https://example.com/font.woff2#latin",
            "https://example.com/archive.tar.gz",
            "https://example.com/audio.opus",
        ] {
            assert!(is_media_asset_url(url), "expected media for {url:?}");
        }
    }

    #[test]
    fn rejects_non_media_urls() {
        for url in [
            "https://example.com/index.html",
            "https://example.com/page",
            "https://example.com/",
            "https://example.com",
            "not-a-url",
            "",
        ] {
            assert!(!is_media_asset_url(url), "expected non-media for {url:?}");
        }
    }

    #[test]
    fn path_helper_matches_url_helper() {
        assert!(is_media_asset_path("/dir/file.png"));
        assert!(is_media_asset_path("file.PNG"));
        assert!(!is_media_asset_path("/page"));
        assert!(!is_media_asset_path(""));
    }

    #[test]
    fn query_and_fragment_are_ignored() {
        assert!(is_media_asset_url(
            "https://example.com/img.jpeg?w=100&h=100"
        ));
        assert!(is_media_asset_url("https://example.com/img.jpeg#anchor"));
        assert!(!is_media_asset_url("https://example.com/page?file=foo.jpg"));
    }
}
