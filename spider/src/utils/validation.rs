use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use lazy_static::lazy_static;

/// Scan only the first bytes (fast + bounded).
pub const PREFIX_SCAN: usize = 2048;

/// If the HTML is huge, avoid loose fallback to reduce false positives.
pub const MAX_LEN_FOR_LOOSE_FALLBACK: usize = 8 * 1024;

/// DataDome tail signature (exact end match after trimming ASCII whitespace).
const DATADOME_END: &[u8] = br#"title="DataDome Device Check"></iframe></html>"#;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
enum Lang {
    En = 0,
    Es = 1,
    Fr = 2,
    De = 3,
    Pt = 4,
    It = 5,
    Nl = 6,
    Ru = 7,
}

#[derive(Copy, Clone, Debug)]
enum PatKind {
    /// Html page.
    Html,
    /// Cloudflare-style "checking your browser..."
    CheckingBrowser,
    Lang(Lang),
    /// Strong, highly-specific markers (tagged).
    TaggedBlock(Lang),
    /// Loose markers (words/phrases). Requires ALSO seeing a 403 marker.
    WordBlock(Lang),
    /// 403 marker (generic).
    Code403,
}

/// Normalize "en-US", "EN_us", "fr-FR" -> Lang (defaults to En).
#[inline]
fn normalize_lang_hint(lang_hint: Option<&str>) -> Lang {
    let s = lang_hint.unwrap_or("en").trim();
    if s.is_empty() {
        return Lang::En;
    }
    let s = s.as_bytes();
    let a = s.get(0).copied().unwrap_or(b'e').to_ascii_lowercase();
    let b = s.get(1).copied().unwrap_or(b'n').to_ascii_lowercase();
    match (a, b) {
        (b'e', b'n') => Lang::En,
        (b'e', b's') => Lang::Es,
        (b'f', b'r') => Lang::Fr,
        (b'd', b'e') => Lang::De,
        (b'p', b't') => Lang::Pt,
        (b'i', b't') => Lang::It,
        (b'n', b'l') => Lang::Nl,
        (b'r', b'u') => Lang::Ru,
        _ => Lang::En,
    }
}

#[inline]
fn lang_bit(l: Lang) -> u16 {
    1u16 << (l as u8)
}

#[inline]
fn trim_ascii_end(mut b: &[u8]) -> &[u8] {
    while let Some(&last) = b.last() {
        if last.is_ascii_whitespace() {
            b = &b[..b.len() - 1];
        } else {
            break;
        }
    }
    b
}

#[inline]
fn ends_with_datadome_device_check(content: Option<&[u8]>) -> bool {
    let bytes = match content {
        Some(b) if !b.is_empty() => b,
        _ => return false,
    };
    let tail = trim_ascii_end(bytes);
    tail.len() >= DATADOME_END.len() && tail.ends_with(DATADOME_END)
}

lazy_static! {
    /// Single source of truth so patterns + kinds can’t drift.
    static ref FALSE_403_RULES: Vec<(&'static str, PatKind)> = {
        let mut r: Vec<(&'static str, PatKind)> = Vec::new();

        // Basic HTML presence gate
        r.push(("<html", PatKind::Html));

        // Cloudflare-ish challenge
        r.push(("checking your browser...", PatKind::CheckingBrowser));

        // Lang hints (two per language: " and ')
        r.extend([
            (r#"lang="en"#, PatKind::Lang(Lang::En)), (r#"lang='en"#, PatKind::Lang(Lang::En)),
            (r#"lang="es"#, PatKind::Lang(Lang::Es)), (r#"lang='es"#, PatKind::Lang(Lang::Es)),
            (r#"lang="fr"#, PatKind::Lang(Lang::Fr)), (r#"lang='fr"#, PatKind::Lang(Lang::Fr)),
            (r#"lang="de"#, PatKind::Lang(Lang::De)), (r#"lang='de"#, PatKind::Lang(Lang::De)),
            (r#"lang="pt"#, PatKind::Lang(Lang::Pt)), (r#"lang='pt"#, PatKind::Lang(Lang::Pt)),
            (r#"lang="it"#, PatKind::Lang(Lang::It)), (r#"lang='it"#, PatKind::Lang(Lang::It)),
            (r#"lang="nl"#, PatKind::Lang(Lang::Nl)), (r#"lang='nl"#, PatKind::Lang(Lang::Nl)),
            (r#"lang="ru"#, PatKind::Lang(Lang::Ru)), (r#"lang='ru"#, PatKind::Lang(Lang::Ru)),
        ]);

        // Generic 403 markers (required for loose fallback)
        r.extend([
            ("<title>403", PatKind::Code403),   // catches "<title>403 Prohibido" etc
            ("<h1>403</h1>", PatKind::Code403), // requested
            (">403<", PatKind::Code403),        // catches <h1 ...>403</h1> too
            (" 403 ", PatKind::Code403),        // broad fallback
            ("\n403\n", PatKind::Code403),      // broad fallback
        ]);

        // Strong/tagged 403/access-denied pages (language-specific)
        r.extend([
            // EN
            ("<title>403 forbidden</title>", PatKind::TaggedBlock(Lang::En)),
            ("<h1>forbidden</h1>", PatKind::TaggedBlock(Lang::En)),
            ("<h1>403 forbidden</h1>", PatKind::TaggedBlock(Lang::En)),
            ("<title>access denied</title>", PatKind::TaggedBlock(Lang::En)),
            ("<h1>access denied</h1>", PatKind::TaggedBlock(Lang::En)),

            // ES
            ("<title>403 prohibido</title>", PatKind::TaggedBlock(Lang::Es)),
            ("<h1>prohibido</h1>", PatKind::TaggedBlock(Lang::Es)),
            ("<title>acceso denegado</title>", PatKind::TaggedBlock(Lang::Es)),
            ("<h1>acceso denegado</h1>", PatKind::TaggedBlock(Lang::Es)),

            // FR
            ("<title>403 interdit</title>", PatKind::TaggedBlock(Lang::Fr)),
            ("<h1>interdit</h1>", PatKind::TaggedBlock(Lang::Fr)),
            ("<title>acces interdit</title>", PatKind::TaggedBlock(Lang::Fr)),
            ("<title>accès interdit</title>", PatKind::TaggedBlock(Lang::Fr)),
            ("<h1>acces interdit</h1>", PatKind::TaggedBlock(Lang::Fr)),
            ("<h1>accès interdit</h1>", PatKind::TaggedBlock(Lang::Fr)),

            // DE
            ("<title>403 verboten</title>", PatKind::TaggedBlock(Lang::De)),
            ("<h1>verboten</h1>", PatKind::TaggedBlock(Lang::De)),
            ("<title>zugriff verweigert</title>", PatKind::TaggedBlock(Lang::De)),
            ("<h1>zugriff verweigert</h1>", PatKind::TaggedBlock(Lang::De)),

            // PT
            ("<title>403 proibido</title>", PatKind::TaggedBlock(Lang::Pt)),
            ("<h1>proibido</h1>", PatKind::TaggedBlock(Lang::Pt)),
            ("<title>acesso negado</title>", PatKind::TaggedBlock(Lang::Pt)),
            ("<h1>acesso negado</h1>", PatKind::TaggedBlock(Lang::Pt)),

            // IT
            ("<title>403 vietato</title>", PatKind::TaggedBlock(Lang::It)),
            ("<h1>vietato</h1>", PatKind::TaggedBlock(Lang::It)),
            ("<title>accesso negato</title>", PatKind::TaggedBlock(Lang::It)),
            ("<h1>accesso negato</h1>", PatKind::TaggedBlock(Lang::It)),

            // NL
            ("<title>403 verboden</title>", PatKind::TaggedBlock(Lang::Nl)),
            ("<h1>verboden</h1>", PatKind::TaggedBlock(Lang::Nl)),

            // RU
            ("<title>доступ запрещен</title>", PatKind::TaggedBlock(Lang::Ru)),
            ("<h1>доступ запрещен</h1>", PatKind::TaggedBlock(Lang::Ru)),
            ("<title>запрещено</title>", PatKind::TaggedBlock(Lang::Ru)),
            ("<h1>запрещено</h1>", PatKind::TaggedBlock(Lang::Ru)),
        ]);

        r.extend([
            ("<title>radware bot manager captcha</title>", PatKind::TaggedBlock(Lang::En)),
            ("radware bot manager captcha", PatKind::TaggedBlock(Lang::En)),
            ("cdn.perfdrive.com/aperture/aperture.js", PatKind::TaggedBlock(Lang::En)),
            ("captcha.perfdrive.com/captcha-public/", PatKind::TaggedBlock(Lang::En)),
            ("validate.perfdrive.com", PatKind::TaggedBlock(Lang::En)),
        ]);

        r.extend([
            // EN
            ("forbidden", PatKind::WordBlock(Lang::En)),
            ("access denied", PatKind::WordBlock(Lang::En)),
            ("access to this resource on the server is denied", PatKind::WordBlock(Lang::En)),

            // ES
            ("prohibido", PatKind::WordBlock(Lang::Es)),
            ("acceso denegado", PatKind::WordBlock(Lang::Es)),

            // FR
            ("interdit", PatKind::WordBlock(Lang::Fr)),
            ("acces interdit", PatKind::WordBlock(Lang::Fr)),
            ("accès interdit", PatKind::WordBlock(Lang::Fr)),

            // DE
            ("verboten", PatKind::WordBlock(Lang::De)),
            ("zugriff verweigert", PatKind::WordBlock(Lang::De)),

            // PT
            ("proibido", PatKind::WordBlock(Lang::Pt)),
            ("acesso negado", PatKind::WordBlock(Lang::Pt)),

            // IT
            ("vietato", PatKind::WordBlock(Lang::It)),
            ("accesso negato", PatKind::WordBlock(Lang::It)),

            // NL
            ("verboden", PatKind::WordBlock(Lang::Nl)),

            // RU
            ("доступ запрещен", PatKind::WordBlock(Lang::Ru)),
            ("запрещено", PatKind::WordBlock(Lang::Ru)),
            ("запрещен", PatKind::WordBlock(Lang::Ru)),
        ]);

        r
    };

    static ref FALSE_403_PATTERNS: Vec<&'static str> = FALSE_403_RULES.iter().map(|(p, _)| *p).collect();
    static ref FALSE_403_KINDS: Vec<PatKind> = FALSE_403_RULES.iter().map(|(_, k)| *k).collect();

    static ref FALSE_403_AC: AhoCorasick = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(MatchKind::LeftmostLongest) // prefer longer matches at same start
        .build(FALSE_403_PATTERNS.as_slice())
        .expect("FALSE_403_AC build");
}

/// True if the body looks like a “false success” block page.
///
/// - DataDome: exact end signature (fast `ends_with`)
/// - Cloudflare challenge: "checking your browser..." in prefix
/// - 403/access denied pages:
///   - strong tagged match OR
///   - (Code403 + word match) fallback (disabled if body > 8k)
#[inline]
pub fn is_false_403(content: Option<&[u8]>, lang_hint: Option<&str>) -> bool {
    if ends_with_datadome_device_check(content) {
        return true;
    }

    let bytes = match content {
        Some(b) if !b.is_empty() => b,
        _ => return false,
    };

    let head = &bytes[..bytes.len().min(PREFIX_SCAN)];

    let mut has_html = false;
    let mut has_checking = false;
    let mut has_403 = false;

    let mut detected_lang: Option<Lang> = None;
    let mut tagged_hits: u16 = 0;
    let mut word_hits: u16 = 0;

    for m in FALSE_403_AC.find_iter(head) {
        let idx = m.pattern().as_usize();
        match FALSE_403_KINDS.get(idx).copied() {
            Some(PatKind::Html) => has_html = true,
            Some(PatKind::CheckingBrowser) => has_checking = true,
            Some(PatKind::Code403) => has_403 = true,
            Some(PatKind::Lang(l)) => {
                if detected_lang.is_none() {
                    detected_lang = Some(l);
                }
            }
            Some(PatKind::TaggedBlock(l)) => tagged_hits |= lang_bit(l),
            Some(PatKind::WordBlock(l)) => word_hits |= lang_bit(l),
            None => {}
        }

        if has_html && (has_checking || tagged_hits != 0 || (has_403 && word_hits != 0)) {
            break;
        }
    }

    if !has_html {
        return false;
    }

    if has_checking {
        return true;
    }

    let effective = detected_lang.unwrap_or_else(|| normalize_lang_hint(lang_hint));
    let lang_mask = lang_bit(effective) | lang_bit(Lang::En);

    // Strong tagged hit (includes Radware/perfdrive markers)
    if (tagged_hits & lang_mask) != 0 {
        return true;
    }

    // Loose fallback only if body isn't huge (optional safety heuristic)
    if bytes.len() > MAX_LEN_FOR_LOOSE_FALLBACK {
        return false;
    }

    has_403 && (word_hits & lang_mask) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn false_for_empty_is_false() {
        assert!(!is_false_403(None, None));
        assert!(!is_false_403(Some(b""), None));
    }

    #[test]
    fn detects_checking_your_browser() {
        let html = br#"<html><title>Checking your browser...</title><body>wait</body></html>"#;
        assert!(is_false_403(Some(html), None));
    }

    #[test]
    fn detects_datadome_tail_end_match() {
        let body = b"random... title=\"DataDome Device Check\"></iframe></html>\n";
        assert!(is_false_403(Some(body), None));
    }

    #[test]
    fn detects_en_title_403() {
        let html = br#"<html><head><title>403 Forbidden</title></head><body>no</body></html>"#;
        assert!(is_false_403(Some(html), None));
    }

    #[test]
    fn detects_es_with_lang() {
        let html = br#"<html lang="es"><head><title>403 Prohibido</title></head></html>"#;
        assert!(is_false_403(Some(html), Some("en")));
    }

    #[test]
    fn detect_failed_403_generic() {
        let body = br###"<html style="height:100%">
<meta name="viewport" content="width=device-width, initial-scale=1, shrink-to-fit=no">
<title> 403 Forbidden
</title>
<div style="height:auto; min-height:100%; ">     <div style="text-align: center; width:800px; margin-left: -400px; position:absolute; top: 30%; left:50%;">
        <h1 style="margin:0; font-size:150px; line-height:150px; font-weight:bold;">403</h1>
<h2 style="margin-top:20px;font-size: 30px;">Forbidden
</h2>
<p>Access to this resource on the server is denied!</p>
</div></div>
</html>"###;
        assert!(is_false_403(Some(body), None));
    }

    #[test]
    fn detects_radware_bot_manager_captcha_title() {
        let body = br#"<html><title>Radware Bot Manager Captcha</title><script async="" src="https://cdn.perfdrive.com/aperture/aperture.js"></script></html>"#;
        assert!(is_false_403(Some(body), None));
    }

    #[test]
    fn prefix_bound_does_not_match_if_beyond_prefix() {
        let mut v = vec![b'a'; PREFIX_SCAN + 128];
        v.extend_from_slice(b"<html><head><title>403 Forbidden</title></head></html>");
        assert!(!is_false_403(Some(&v), None));
    }
}
