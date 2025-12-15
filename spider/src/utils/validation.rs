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
    /// Check browser.
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
    static ref FALSE_403_PATTERNS: Vec<&'static str> = {
        let mut p = Vec::new();

        p.push("<html");

        p.push("checking your browser...");

        p.extend([
            r#"lang="en"#, r#"lang='en"#,
            r#"lang="es"#, r#"lang='es"#,
            r#"lang="fr"#, r#"lang='fr"#,
            r#"lang="de"#, r#"lang='de"#,
            r#"lang="pt"#, r#"lang='pt"#,
            r#"lang="it"#, r#"lang='it"#,
            r#"lang="nl"#, r#"lang='nl"#,
            r#"lang="ru"#, r#"lang='ru"#,
        ]);

        p.extend([
            "<title>403",     // catches "<title>403 Prohibido" etc
            "<h1>403</h1>",   // requested
            ">403<",          // catches <h1 ...>403</h1> too
            " 403 ",          // broad fallback
            "\n403\n",        // broad fallback
        ]);

        p.extend([
            // EN
            "<title>403 forbidden</title>",
            "<h1>forbidden</h1>",
            "<h1>403 forbidden</h1>",
            "<title>access denied</title>",
            "<h1>access denied</h1>",

            // ES
            "<title>403 prohibido</title>",
            "<h1>prohibido</h1>",
            "<title>acceso denegado</title>",
            "<h1>acceso denegado</h1>",

            // FR
            "<title>403 interdit</title>",
            "<h1>interdit</h1>",
            "<title>acces interdit</title>",
            "<title>accès interdit</title>",
            "<h1>acces interdit</h1>",
            "<h1>accès interdit</h1>",

            // DE
            "<title>403 verboten</title>",
            "<h1>verboten</h1>",
            "<title>zugriff verweigert</title>",
            "<h1>zugriff verweigert</h1>",

            // PT
            "<title>403 proibido</title>",
            "<h1>proibido</h1>",
            "<title>acesso negado</title>",
            "<h1>acesso negado</h1>",

            // IT
            "<title>403 vietato</title>",
            "<h1>vietato</h1>",
            "<title>accesso negato</title>",
            "<h1>accesso negato</h1>",

            // NL
            "<title>403 verboden</title>",
            "<h1>verboden</h1>",

            // RU
            "<title>доступ запрещен</title>",
            "<h1>доступ запрещен</h1>",
            "<title>запрещено</title>",
            "<h1>запрещено</h1>",
        ]);

        p.extend([
            // EN
            "forbidden",
            "access denied",
            "access to this resource on the server is denied",

            // ES
            "prohibido",
            "acceso denegado",

            // FR
            "interdit",
            "acces interdit",
            "accès interdit",

            // DE
            "verboten",
            "zugriff verweigert",

            // PT
            "proibido",
            "acesso negado",

            // IT
            "vietato",
            "accesso negato",

            // NL
            "verboden",

            // RU
            "доступ запрещен",
            "запрещено",
            "запрещен",
        ]);

        p
    };

    static ref FALSE_403_KINDS: Vec<PatKind> = {
        let mut k = Vec::new();

        k.push(PatKind::Html);

        k.push(PatKind::CheckingBrowser);

        let langs = [Lang::En, Lang::Es, Lang::Fr, Lang::De, Lang::Pt, Lang::It, Lang::Nl, Lang::Ru];
        for &l in &langs {
            k.push(PatKind::Lang(l));
            k.push(PatKind::Lang(l));
        }

        k.extend([PatKind::Code403; 5]);

        k.extend([PatKind::TaggedBlock(Lang::En); 5]);
        k.extend([PatKind::TaggedBlock(Lang::Es); 4]);
        k.extend([PatKind::TaggedBlock(Lang::Fr); 6]);
        k.extend([PatKind::TaggedBlock(Lang::De); 4]);
        k.extend([PatKind::TaggedBlock(Lang::Pt); 4]);
        k.extend([PatKind::TaggedBlock(Lang::It); 4]);
        k.extend([PatKind::TaggedBlock(Lang::Nl); 2]);
        k.extend([PatKind::TaggedBlock(Lang::Ru); 4]);

        k.extend([PatKind::WordBlock(Lang::En); 3]);
        k.extend([PatKind::WordBlock(Lang::Es); 2]);
        k.extend([PatKind::WordBlock(Lang::Fr); 3]);
        k.extend([PatKind::WordBlock(Lang::De); 2]);
        k.extend([PatKind::WordBlock(Lang::Pt); 2]);
        k.extend([PatKind::WordBlock(Lang::It); 2]);
        k.extend([PatKind::WordBlock(Lang::Nl); 1]);
        k.extend([PatKind::WordBlock(Lang::Ru); 3]);

        debug_assert_eq!(k.len(), FALSE_403_PATTERNS.len());
        k
    };

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

        // Early exit when decisive.
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

    // Strong tagged hit
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
    fn prefix_bound_does_not_match_if_beyond_prefix() {
        let mut v = vec![b'a'; PREFIX_SCAN + 128];
        v.extend_from_slice(b"<html><head><title>403 Forbidden</title></head></html>");
        assert!(!is_false_403(Some(&v), None));
    }
}
