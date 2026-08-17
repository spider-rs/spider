//! HTML cleaning utilities for automation.
//!
//! Provides multiple cleaning levels for preparing HTML content
//! before sending to LLM models.

use lol_html::html_content::Element;
use lol_html::{doc_comments, rewrite_str, ElementContentHandlers, RewriteStrSettings, Selector};
use spider_agent_types::{CleaningIntent, ContentAnalysis, HtmlCleaningProfile};
use std::borrow::Cow;
use std::sync::LazyLock;

/// Declare a pre-parsed CSS selector shared by every cleaning call.
///
/// Cleaning runs over the full page HTML on every page/round, and the
/// `element!` macro re-parses its selector string on each invocation —
/// hoisting the parsed [`Selector`]s makes that a one-time cost.
macro_rules! static_selector {
    ($name:ident, $selector:literal) => {
        static $name: LazyLock<Selector> =
            LazyLock::new(|| $selector.parse().expect("valid selector"));
    };
}

static_selector!(SEL_SCRIPT, "script");
static_selector!(SEL_STYLE, "style");
static_selector!(SEL_LINK, "link");
static_selector!(SEL_IFRAME, "iframe");
static_selector!(SEL_SVG, "svg");
static_selector!(SEL_NOSCRIPT, "noscript");
static_selector!(SEL_CANVAS, "canvas");
static_selector!(SEL_VIDEO, "video");
static_selector!(SEL_NAV, "nav");
static_selector!(SEL_FOOTER, "footer");
static_selector!(SEL_IMG, "img");
static_selector!(SEL_PICTURE, "picture");
static_selector!(SEL_META, "meta");
static_selector!(SEL_ANY, "*");
static_selector!(SEL_DISPLAY_NONE, "[style*='display:none']");
static_selector!(SEL_ID_AD, "[id*='ad']");
static_selector!(SEL_CLASS_AD, "[class*='ad']");
static_selector!(SEL_ID_TRACKING, "[id*='tracking']");
static_selector!(SEL_CLASS_TRACKING, "[class*='tracking']");

/// Handler entry type used by the cleaning rewrites.
type Handler = (Cow<'static, Selector>, ElementContentHandlers<'static>);

/// Build a handler that removes every element matching `sel`.
fn removed(sel: &'static Selector) -> Handler {
    (
        Cow::Borrowed(sel),
        ElementContentHandlers::default().element(|el: &mut Element| {
            el.remove();
            Ok(())
        }),
    )
}

/// Build the `meta` handler shared by base/slim cleaning: keep only
/// `name="title"` and `name="description"` meta tags.
fn meta_title_description() -> Handler {
    (
        Cow::Borrowed(&*SEL_META),
        ElementContentHandlers::default().element(|el: &mut Element| {
            if let Some(attribute) = el.get_attribute("name") {
                if attribute != "title" && attribute != "description" {
                    el.remove();
                }
            } else {
                el.remove();
            }
            Ok(())
        }),
    )
}

/// Handler that removes document comments.
fn comment_remover() -> Vec<lol_html::DocumentContentHandlers<'static>> {
    vec![doc_comments!(|c| {
        c.remove();
        Ok(())
    })]
}

/// Raw passthrough - no cleaning.
pub fn clean_html_raw(html: &str) -> String {
    html.to_string()
}

/// Clean the HTML removing CSS and JS (base level).
///
/// Removes:
/// - `<script>` tags
/// - `<style>` tags
/// - `<link>` tags
/// - `<iframe>` tags
/// - Elements with display:none
/// - Ad and tracking elements
/// - Non-essential meta tags
pub fn clean_html_base(html: &str) -> String {
    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                removed(&SEL_SCRIPT),
                removed(&SEL_STYLE),
                removed(&SEL_LINK),
                removed(&SEL_IFRAME),
                removed(&SEL_DISPLAY_NONE),
                removed(&SEL_ID_AD),
                removed(&SEL_CLASS_AD),
                removed(&SEL_ID_TRACKING),
                removed(&SEL_CLASS_TRACKING),
                meta_title_description(),
            ],
            document_content_handlers: comment_remover(),
            ..RewriteStrSettings::new()
        },
    ) {
        Ok(r) => r,
        _ => html.into(),
    }
}

/// Slim HTML cleaning - removes heavy elements.
///
/// In addition to base cleaning, removes:
/// - `<svg>` tags
/// - `<noscript>` tags
/// - `<canvas>` tags
/// - `<video>` tags
/// - Base64 images
/// - Picture elements with data URIs
pub fn clean_html_slim(html: &str) -> String {
    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                removed(&SEL_SCRIPT),
                removed(&SEL_STYLE),
                removed(&SEL_SVG),
                removed(&SEL_NOSCRIPT),
                removed(&SEL_LINK),
                removed(&SEL_IFRAME),
                removed(&SEL_CANVAS),
                removed(&SEL_VIDEO),
                (
                    Cow::Borrowed(&*SEL_IMG),
                    ElementContentHandlers::default().element(|el: &mut Element| {
                        if let Some(src) = el.get_attribute("src") {
                            if src.starts_with("data:image") {
                                el.remove();
                            }
                        }
                        Ok(())
                    }),
                ),
                (
                    Cow::Borrowed(&*SEL_PICTURE),
                    ElementContentHandlers::default().element(|el: &mut Element| {
                        // Remove if it contains data URIs
                        if let Some(src) = el.get_attribute("src") {
                            if src.starts_with("data:") {
                                el.remove();
                            }
                        }
                        Ok(())
                    }),
                ),
                removed(&SEL_DISPLAY_NONE),
                removed(&SEL_ID_AD),
                removed(&SEL_CLASS_AD),
                removed(&SEL_ID_TRACKING),
                removed(&SEL_CLASS_TRACKING),
                meta_title_description(),
            ],
            document_content_handlers: comment_remover(),
            ..RewriteStrSettings::new()
        },
    ) {
        Ok(r) => r,
        _ => html.into(),
    }
}

/// Full/aggressive HTML cleaning.
///
/// In addition to other cleaning levels, also removes:
/// - `<nav>` tags
/// - `<footer>` tags
/// - Most attributes except id, class, and data-*
pub fn clean_html_full(html: &str) -> String {
    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![
                removed(&SEL_SCRIPT),
                removed(&SEL_STYLE),
                removed(&SEL_SVG),
                removed(&SEL_NAV),
                removed(&SEL_FOOTER),
                removed(&SEL_NOSCRIPT),
                removed(&SEL_LINK),
                removed(&SEL_IFRAME),
                removed(&SEL_CANVAS),
                removed(&SEL_VIDEO),
                (
                    Cow::Borrowed(&*SEL_META),
                    ElementContentHandlers::default().element(|el: &mut Element| {
                        // ASCII case-insensitive compare instead of allocating a
                        // lowercased copy per element. Exactly equivalent here:
                        // no non-ASCII char lowercases into the letters of
                        // "viewport" or "charset" (the only such mapping is
                        // U+212A -> 'k', which neither contains).
                        let keep = el.get_attribute("name").is_some_and(|n| {
                            n.eq_ignore_ascii_case("viewport") || n.eq_ignore_ascii_case("charset")
                        });
                        if !keep {
                            el.remove();
                        }
                        Ok(())
                    }),
                ),
                (
                    Cow::Borrowed(&*SEL_ANY),
                    ElementContentHandlers::default().element(|el: &mut Element| {
                        // Keep only: id, class, data-*
                        let mut to_remove: Vec<String> = Vec::new();
                        for attr in el.attributes().iter() {
                            let n = attr.name();
                            let keep = n == "id" || n == "class" || n.starts_with("data-");
                            if !keep {
                                to_remove.push(n);
                            }
                        }
                        for attr in to_remove {
                            el.remove_attribute(&attr);
                        }
                        Ok(())
                    }),
                ),
            ],
            document_content_handlers: comment_remover(),
            ..RewriteStrSettings::new()
        },
    ) {
        Ok(r) => r,
        _ => html.into(),
    }
}

/// Default cleaner (base level).
#[inline]
pub fn clean_html(html: &str) -> String {
    clean_html_base(html)
}

/// Clean HTML using a specific profile.
pub fn clean_html_with_profile(html: &str, profile: HtmlCleaningProfile) -> String {
    clean_html_with_profile_and_intent(html, profile, CleaningIntent::General)
}

/// Clean HTML with a specific profile and intent.
///
/// The intent helps Auto mode choose the right cleaning level:
/// - `Extraction` - can be more aggressive, removes nav/footer
/// - `Action` - preserves interactive elements
/// - `General` - balanced approach
pub fn clean_html_with_profile_and_intent(
    html: &str,
    profile: HtmlCleaningProfile,
    intent: CleaningIntent,
) -> String {
    match profile {
        HtmlCleaningProfile::Raw => clean_html_raw(html),
        HtmlCleaningProfile::Default => clean_html(html),
        HtmlCleaningProfile::Aggressive => clean_html_full(html),
        HtmlCleaningProfile::Slim => clean_html_slim(html),
        HtmlCleaningProfile::Minimal => clean_html_base(html),
        HtmlCleaningProfile::Auto => {
            // Analyze content and choose the best profile based on intent
            let analysis = ContentAnalysis::analyze(html);
            let auto_profile =
                HtmlCleaningProfile::from_content_analysis_with_intent(&analysis, intent);
            // Recursively call with determined profile (won't be Auto again)
            clean_html_with_profile_and_intent(html, auto_profile, intent)
        }
    }
}

/// Smart HTML cleaner that automatically determines the best cleaning level.
///
/// This is the recommended function for cleaning HTML when you don't have
/// a specific profile preference. It analyzes the content and chooses
/// the optimal cleaning level based on:
/// - Content size and text ratio
/// - Presence of heavy elements (SVGs, canvas, video)
/// - The intended use case (extraction vs action)
pub fn smart_clean_html(html: &str, intent: CleaningIntent) -> String {
    clean_html_with_profile_and_intent(html, HtmlCleaningProfile::Auto, intent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_html_raw() {
        let html = "<script>alert(1)</script><p>Hello</p>";
        assert_eq!(clean_html_raw(html), html);
    }

    #[test]
    fn test_clean_html_base() {
        let html = "<script>alert(1)</script><p>Hello</p><style>.x{}</style>";
        let cleaned = clean_html_base(html);
        assert!(!cleaned.contains("<script>"));
        assert!(!cleaned.contains("<style>"));
        assert!(cleaned.contains("<p>Hello</p>"));
    }

    #[test]
    fn test_clean_html_slim() {
        let html = "<svg><path/></svg><p>Hello</p><canvas></canvas>";
        let cleaned = clean_html_slim(html);
        assert!(!cleaned.contains("<svg>"));
        assert!(!cleaned.contains("<canvas>"));
        assert!(cleaned.contains("<p>Hello</p>"));
    }

    #[test]
    fn test_clean_html_full() {
        let html = "<nav>Menu</nav><p>Hello</p><footer>Footer</footer>";
        let cleaned = clean_html_full(html);
        assert!(!cleaned.contains("<nav>"));
        assert!(!cleaned.contains("<footer>"));
        assert!(cleaned.contains("<p>Hello</p>"));
    }

    #[test]
    fn test_smart_clean_html() {
        // Small, simple content should use minimal cleaning
        let simple = "<html><body><p>Hello World!</p></body></html>";
        let _cleaned = smart_clean_html(simple, CleaningIntent::General);
        // Just verify it doesn't panic
    }

    // ── Golden byte-identity tests ───────────────────────────────────────
    //
    // The expected strings below were captured verbatim from the cleaning
    // implementation BEFORE the static-selector refactor. Any byte drift in
    // cleaning output is a regression: downstream consumers (diffing, caches,
    // stagnation hashing) rely on stable output for identical input.

    /// Representative inputs exercising every handler across all levels.
    fn golden_inputs() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "kitchen_sink",
                r##"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta name="description" content="desc"><meta name="robots" content="noindex"><meta property="og:title" content="x"><title>T</title><link rel="stylesheet" href="a.css"><style>.x{color:red}</style><script src="a.js"></script></head><body onload="x()"><!-- comment --><nav id="menu"><a href="/">Home</a></nav><div id="sidebar-ad" class="widget">AD</div><span class="tracking-pixel"></span><div style="display:none">hidden</div><iframe src="f.html"></iframe><svg viewBox="0 0 1 1"><path d="M0 0"/></svg><noscript>enable js</noscript><canvas width="1"></canvas><video src="v.mp4"></video><img src="data:image/png;base64,AAAA" alt="b64"><img src="real.png" alt="ok" width="5"><picture src="data:foo"><source srcset="x.webp"></picture><p data-test="keep" title="drop" id="p1" class="c1">Hello <b>World</b></p><footer>foot</footer><script>var y=1;</script></body></html>"##,
            ),
            (
                "case_variants",
                r##"<HTML><HEAD><META NAME="Description" CONTENT="d"><META name="VIEWPORT" content="w"><SCRIPT>x</SCRIPT><STYLE>y</STYLE></HEAD><BODY><NAV>n</NAV><SVG><circle/></SVG><P STYLE="Display:None">t</P><DIV ID="AdBanner">a</DIV><p>keep</p></BODY></HTML>"##,
            ),
            (
                "malformed",
                r##"<div><p>unclosed<script>let a = "<div>"</script><span class="load">tail</span>"##,
            ),
            (
                "attr_edge",
                r##"<div id="header" class="shadow" data-x="1" data-foo-bar="2" aria-hidden="true" onclick="go()" style="color:blue"><a href="/x" target="_blank" rel="nofollow" id="l" class="lnk" data-nav="y">x</a><meta name="title" content="t"><meta content="bare"></div>"##,
            ),
            ("empty", ""),
            (
                "plain_text",
                "just some text, no tags & entities &amp; <notatag",
            ),
            (
                "unicode_meta",
                r##"<meta name="VİEWPORT" content="turkish-dotted-I"><meta name="viewport" content="ok"><p>é–ü—中文</p>"##,
            ),
        ]
    }

    /// (name, level) -> expected output captured from the pre-refactor code.
    fn golden_expected(name: &str, level: &str) -> &'static str {
        match (name, level) {
            ("kitchen_sink", "base") => {
                r##"<!DOCTYPE html><html lang="en"><head><meta name="description" content="desc"><title>T</title></head><body onload="x()"><nav id="menu"><a href="/">Home</a></nav><svg viewBox="0 0 1 1"><path d="M0 0"/></svg><noscript>enable js</noscript><canvas width="1"></canvas><video src="v.mp4"></video><img src="data:image/png;base64,AAAA" alt="b64"><img src="real.png" alt="ok" width="5"><picture src="data:foo"><source srcset="x.webp"></picture><p data-test="keep" title="drop" id="p1" class="c1">Hello <b>World</b></p><footer>foot</footer></body></html>"##
            }
            ("kitchen_sink", "slim") => {
                r##"<!DOCTYPE html><html lang="en"><head><meta name="description" content="desc"><title>T</title></head><body onload="x()"><nav id="menu"><a href="/">Home</a></nav><img src="real.png" alt="ok" width="5"><p data-test="keep" title="drop" id="p1" class="c1">Hello <b>World</b></p><footer>foot</footer></body></html>"##
            }
            ("kitchen_sink", "full") => {
                r##"<!DOCTYPE html><html><head><meta><title>T</title></head><body><div id="sidebar-ad" class="widget">AD</div><span class="tracking-pixel"></span><div>hidden</div><img><img><picture><source></picture><p data-test="keep" id="p1" class="c1">Hello <b>World</b></p></body></html>"##
            }
            ("case_variants", "base") => {
                r##"<HTML><HEAD></HEAD><BODY><NAV>n</NAV><SVG><circle/></SVG><P STYLE="Display:None">t</P><DIV ID="AdBanner">a</DIV><p>keep</p></BODY></HTML>"##
            }
            ("case_variants", "slim") => {
                r##"<HTML><HEAD></HEAD><BODY><NAV>n</NAV><P STYLE="Display:None">t</P><DIV ID="AdBanner">a</DIV><p>keep</p></BODY></HTML>"##
            }
            ("case_variants", "full") => {
                r##"<HTML><HEAD><META></HEAD><BODY><P>t</P><DIV ID="AdBanner">a</DIV><p>keep</p></BODY></HTML>"##
            }
            ("malformed", "base") | ("malformed", "slim") => "<div><p>unclosed",
            ("malformed", "full") => r##"<div><p>unclosed<span class="load">tail</span>"##,
            // "header" contains "ad" -> the whole [id*='ad'] div is removed.
            ("attr_edge", "base") | ("attr_edge", "slim") => "",
            ("attr_edge", "full") => {
                r##"<div id="header" class="shadow" data-x="1" data-foo-bar="2"><a id="l" class="lnk" data-nav="y">x</a></div>"##
            }
            ("empty", _) => "",
            ("plain_text", _) => "just some text, no tags & entities &amp; <notatag",
            ("unicode_meta", "base") | ("unicode_meta", "slim") => "<p>é–ü—中文</p>",
            // Turkish dotted İ is not an ASCII "viewport" -> that meta is
            // removed; the plain-ASCII one is kept (attributes stripped).
            ("unicode_meta", "full") => "<meta><p>é–ü—中文</p>",
            _ => unreachable!("unknown golden case {name}/{level}"),
        }
    }

    #[test]
    fn golden_outputs_are_byte_identical() {
        for (name, html) in golden_inputs() {
            for (level, out) in [
                ("base", clean_html_base(html)),
                ("slim", clean_html_slim(html)),
                ("full", clean_html_full(html)),
            ] {
                assert_eq!(
                    out,
                    golden_expected(name, level),
                    "cleaning output drifted for {name}/{level}"
                );
            }
        }
    }
}
