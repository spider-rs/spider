use percent_encoding::percent_decode_str;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

/// Windows reserved device names (case-insensitive, no extension)
fn is_windows_reserved_name(name: &str) -> bool {
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let upper = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    RESERVED.contains(&upper.as_str())
}

fn cap_component(s: String, max_len: usize) -> String {
    if s.len() <= max_len {
        return s;
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    let h = hasher.finish();
    let keep = max_len.saturating_sub(1 + 8);
    let mut base = s.chars().take(keep).collect::<String>();
    base.push('~');
    base.push_str(&format!("{:08x}", (h as u32)));
    base
}

/// Decode %XX, normalize NFC, and sanitize to a safe file system component.
/// Returns None for empty / "." / ".." segments so callers can skip them.
fn sanitize_component(raw: &str) -> Option<String> {
    // Skip empty / dot segments (often created by leading/trailing/double slashes)
    if raw.is_empty() || raw == "." || raw == ".." {
        return None;
    }

    let decoded = percent_decode_str(raw).decode_utf8_lossy();
    let normalized: Cow<str> = Cow::Owned(decoded.nfc().collect::<String>());

    // Remove/replace forbidden characters.
    let mut out = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        let bad = matches!(
            ch,
            '\0' | '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'
        ) || ch.is_control();
        out.push(if bad { '_' } else { ch });
    }

    // Trim spaces and dots (Windows unsafe at ends)
    let out = out.trim_matches([' ', '.']).to_string();

    // If it became empty after trimming, drop it
    if out.is_empty() {
        return None;
    }

    // Avoid Windows reserved names
    let out = if is_windows_reserved_name(&out) {
        format!("{}_file", out)
    } else {
        out
    };

    Some(cap_component(out, 120))
}

/// If `leaf` has an extension, keep it; else use "index.html" or "{leaf}.html".
fn choose_filename(leaf: &str, has_trailing_slash: bool) -> String {
    if has_trailing_slash || leaf.is_empty() || !leaf.contains('.') {
        if leaf.is_empty() {
            "index.html".to_string()
        } else {
            format!("{}.html", leaf)
        }
    } else {
        leaf.to_string()
    }
}

/// Build a safe local path from a URL path.
pub fn build_local_path(base: &Path, url_path: &str) -> PathBuf {
    let has_trailing_slash = url_path.ends_with('/');

    // Split raw segments and DROP empties (from leading/trailing/double slashes)
    let raw_segments = url_path.split('/').filter(|s| !s.is_empty());

    // Sanitize only the meaningful segments
    let mut clean: Vec<String> = raw_segments.filter_map(sanitize_component).collect();

    // If nothing meaningful remains, write index.html at base
    if clean.is_empty() {
        let mut p = base.to_path_buf();
        p.push("index.html");
        return p;
    }

    // Determine filename
    let leaf_raw = clean.pop().unwrap_or_default();
    let filename = choose_filename(&leaf_raw, has_trailing_slash);

    // Rebuild path
    let mut path = base.to_path_buf();
    for dir in clean {
        path.push(dir);
    }
    path.push(filename);
    path
}
