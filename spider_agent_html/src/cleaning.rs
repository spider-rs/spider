//! HTML cleaning utilities for automation.
//!
//! Provides multiple cleaning levels for preparing HTML content
//! before sending to LLM models.

use lol_html::{doc_comments, element, rewrite_str, RewriteStrSettings};
use spider_agent_types::{CleaningIntent, ContentAnalysis, HtmlCleaningProfile};

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
                element!("script", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("link", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("iframe", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[style*='display:none']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    if let Some(attribute) = el.get_attribute("name") {
                        if attribute != "title" && attribute != "description" {
                            el.remove();
                        }
                    } else {
                        el.remove();
                    }
                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
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
                element!("script", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("svg", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("noscript", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("link", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("iframe", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("canvas", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("video", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("img", |el| {
                    if let Some(src) = el.get_attribute("src") {
                        if src.starts_with("data:image") {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                element!("picture", |el| {
                    // Remove if it contains data URIs
                    if let Some(src) = el.get_attribute("src") {
                        if src.starts_with("data:") {
                            el.remove();
                        }
                    }
                    Ok(())
                }),
                element!("[style*='display:none']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='ad']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[id*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("[class*='tracking']", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    if let Some(attribute) = el.get_attribute("name") {
                        if attribute != "title" && attribute != "description" {
                            el.remove();
                        }
                    } else {
                        el.remove();
                    }
                    Ok(())
                }),
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
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
                element!("script", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("style", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("svg", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("nav", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("footer", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("noscript", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("link", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("iframe", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("canvas", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("video", |el| {
                    el.remove();
                    Ok(())
                }),
                element!("meta", |el| {
                    let name = el.get_attribute("name").map(|n| n.to_lowercase());
                    if !matches!(name.as_deref(), Some("viewport") | Some("charset")) {
                        el.remove();
                    }
                    Ok(())
                }),
                element!("*", |el| {
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
            ],
            document_content_handlers: vec![doc_comments!(|c| {
                c.remove();
                Ok(())
            })],
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
}
