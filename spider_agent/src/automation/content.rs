//! Content analysis for smart automation decisions.
//!
//! Analyzes HTML content to determine:
//! - Whether screenshots are needed for extraction
//! - Optimal cleaning profile for the content
//! - Content type and complexity
//!
//! Uses Aho-Corasick algorithm for efficient multi-pattern matching.

use aho_corasick::AhoCorasick;
use std::sync::LazyLock;

/// Aho-Corasick pattern matcher for visual element tags (case-insensitive).
static VISUAL_ELEMENT_MATCHER: LazyLock<AhoCorasick> = LazyLock::new(|| {
    AhoCorasick::builder()
        .ascii_case_insensitive(true)
        .build(&["<iframe", "<video", "<canvas", "<embed", "<object"])
        .expect("valid patterns")
});

/// Aho-Corasick pattern matcher for SPA framework indicators.
static SPA_INDICATOR_MATCHER: LazyLock<AhoCorasick> = LazyLock::new(|| {
    AhoCorasick::builder()
        .ascii_case_insensitive(true)
        .build(&[
            "data-reactroot",
            "__next",
            "id=\"app\"",
            "id=\"root\"",
            "ng-app",
            "v-app",
            "data-v-",
        ])
        .expect("valid patterns")
});

/// Aho-Corasick pattern matcher for SVG tags.
static SVG_MATCHER: LazyLock<AhoCorasick> = LazyLock::new(|| {
    AhoCorasick::builder()
        .ascii_case_insensitive(true)
        .build(&["<svg"])
        .expect("valid patterns")
});

/// Result of analyzing HTML content.
///
/// Helps decide whether to rely on HTML text alone or require
/// a screenshot for accurate extraction.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContentAnalysis {
    /// Whether the content is "thin" (low text content).
    pub is_thin_content: bool,
    /// Whether visual elements that need screenshot were detected.
    pub has_visual_elements: bool,
    /// Whether dynamic content indicators were found.
    pub has_dynamic_content: bool,
    /// Recommendation: true if screenshot is recommended.
    pub needs_screenshot: bool,

    // Element counts
    /// Count of iframe elements.
    pub iframe_count: usize,
    /// Count of video elements.
    pub video_count: usize,
    /// Count of canvas elements.
    pub canvas_count: usize,
    /// Count of embed/object elements.
    pub embed_count: usize,
    /// Count of SVG elements.
    pub svg_count: usize,

    // Size metrics
    /// Approximate visible text length.
    pub text_length: usize,
    /// Total HTML length.
    pub html_length: usize,
    /// Ratio of text to HTML.
    pub text_ratio: f32,

    // Byte size tracking
    /// Total bytes of SVG elements.
    pub svg_bytes: usize,
    /// Total bytes of script elements.
    pub script_bytes: usize,
    /// Total bytes of style elements.
    pub style_bytes: usize,
    /// Total bytes of base64-encoded data.
    pub base64_bytes: usize,
    /// Total bytes that could be cleaned.
    pub cleanable_bytes: usize,
    /// Ratio of cleanable bytes to total.
    pub cleanable_ratio: f32,

    /// Indicators found (for debugging).
    #[serde(default)]
    pub indicators: Vec<String>,
}

impl ContentAnalysis {
    /// Minimum text length to consider content "substantial".
    const MIN_TEXT_LENGTH: usize = 200;
    /// Text-to-HTML ratio below which content is considered "thin".
    const MIN_TEXT_RATIO: f32 = 0.05;

    /// Analyze HTML content (fast mode).
    pub fn analyze(html: &str) -> Self {
        Self::analyze_internal(html, false)
    }

    /// Analyze HTML content with full byte size calculation.
    pub fn analyze_full(html: &str) -> Self {
        Self::analyze_internal(html, true)
    }

    fn analyze_internal(html: &str, calculate_sizes: bool) -> Self {
        let html_bytes = html.as_bytes();
        let html_length = html.len();

        let mut analysis = Self {
            html_length,
            ..Default::default()
        };

        // Count visual elements using Aho-Corasick (single pass, case-insensitive)
        for mat in VISUAL_ELEMENT_MATCHER.find_iter(html_bytes) {
            match mat.pattern().as_usize() {
                0 => analysis.iframe_count += 1,    // <iframe
                1 => analysis.video_count += 1,     // <video
                2 => analysis.canvas_count += 1,    // <canvas
                3 | 4 => analysis.embed_count += 1, // <embed, <object
                _ => {}
            }
        }

        // Count SVGs using Aho-Corasick
        analysis.svg_count = SVG_MATCHER.find_iter(html_bytes).count();

        // Check for SPA indicators using Aho-Corasick
        analysis.has_dynamic_content = SPA_INDICATOR_MATCHER.find(html_bytes).is_some();

        // Estimate text length
        analysis.text_length = estimate_text_length(html);

        // Calculate byte sizes
        if calculate_sizes {
            analysis.svg_bytes = estimate_tag_bytes(html, "svg");
            analysis.script_bytes = estimate_tag_bytes(html, "script");
            analysis.style_bytes = estimate_tag_bytes(html, "style");
            analysis.base64_bytes = estimate_base64_bytes(html);
        } else {
            // Fast estimation using heuristics
            analysis.svg_bytes = analysis.svg_count * 5_000;
            analysis.script_bytes = count_script_tags_fast(html_bytes) * 10_000;
            analysis.style_bytes = count_style_tags_fast(html_bytes) * 2_000;
            analysis.base64_bytes = estimate_base64_bytes_fast(html_bytes);
        }

        analysis.cleanable_bytes = analysis.svg_bytes
            + analysis.script_bytes
            + analysis.style_bytes
            + analysis.base64_bytes;

        // Calculate ratios
        analysis.text_ratio = if html_length > 0 {
            analysis.text_length as f32 / html_length as f32
        } else {
            0.0
        };

        analysis.cleanable_ratio = if html_length > 0 {
            analysis.cleanable_bytes as f32 / html_length as f32
        } else {
            0.0
        };

        // Determine if content is thin
        analysis.is_thin_content = analysis.text_length < Self::MIN_TEXT_LENGTH
            || analysis.text_ratio < Self::MIN_TEXT_RATIO;

        // Determine if visual elements present
        analysis.has_visual_elements = analysis.iframe_count > 0
            || analysis.video_count > 0
            || analysis.canvas_count > 0
            || analysis.embed_count > 0;

        // Add indicators
        if analysis.is_thin_content {
            analysis.indicators.push("thin_content".to_string());
        }
        if analysis.has_visual_elements {
            analysis.indicators.push("visual_elements".to_string());
        }
        if analysis.has_dynamic_content {
            analysis.indicators.push("dynamic_content".to_string());
        }

        // Determine if screenshot needed
        analysis.needs_screenshot = analysis.is_thin_content
            || analysis.has_visual_elements
            || (analysis.has_dynamic_content && analysis.text_ratio < 0.1);

        analysis
    }

    /// Quick check if screenshot is needed (inline, no full analysis).
    ///
    /// Uses Aho-Corasick for efficient multi-pattern matching without
    /// allocating memory for lowercase conversion.
    #[inline]
    pub fn quick_needs_screenshot(html: &str) -> bool {
        let bytes = html.as_bytes();

        // Quick check for visual elements using Aho-Corasick
        if VISUAL_ELEMENT_MATCHER.find(bytes).is_some() {
            return true;
        }

        // Very short HTML likely needs screenshot
        if html.len() < 1000 {
            return true;
        }

        // Check for SPA indicators with thin content
        if SPA_INDICATOR_MATCHER.find(bytes).is_some() {
            let text_len = estimate_text_length(html);
            if text_len < 200 {
                return true;
            }
        }

        false
    }

    /// Check if HTML has any visual elements (iframe, video, canvas, embed, object).
    #[inline]
    pub fn has_visual_elements_quick(html: &str) -> bool {
        VISUAL_ELEMENT_MATCHER.find(html.as_bytes()).is_some()
    }

    /// Get recommended cleaning profile based on analysis.
    pub fn recommended_cleaning(&self) -> super::HtmlCleaningProfile {
        use super::HtmlCleaningProfile;

        if self.cleanable_ratio > 0.5 {
            // More than half is cleanable - aggressive
            HtmlCleaningProfile::Aggressive
        } else if self.svg_bytes > 50_000 || self.base64_bytes > 50_000 {
            // Heavy SVGs or base64 - slim
            HtmlCleaningProfile::Slim
        } else if self.has_dynamic_content {
            // SPA - preserve some structure
            HtmlCleaningProfile::Minimal
        } else if self.is_thin_content {
            // Little text - be careful with cleaning
            HtmlCleaningProfile::Minimal
        } else {
            // Normal content
            HtmlCleaningProfile::Default
        }
    }

    /// Get a summary string.
    pub fn summary(&self) -> String {
        format!(
            "text={}, html={}, ratio={:.2}, cleanable={:.0}%, screenshot={}",
            self.text_length,
            self.html_length,
            self.text_ratio,
            self.cleanable_ratio * 100.0,
            self.needs_screenshot
        )
    }
}

/// Fast estimate of script tag count.
#[inline]
fn count_script_tags_fast(html: &[u8]) -> usize {
    static SCRIPT_MATCHER: LazyLock<AhoCorasick> = LazyLock::new(|| {
        AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&["<script"])
            .expect("valid patterns")
    });
    SCRIPT_MATCHER.find_iter(html).count()
}

/// Fast estimate of style tag count.
#[inline]
fn count_style_tags_fast(html: &[u8]) -> usize {
    static STYLE_MATCHER: LazyLock<AhoCorasick> = LazyLock::new(|| {
        AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&["<style"])
            .expect("valid patterns")
    });
    STYLE_MATCHER.find_iter(html).count()
}

/// Estimate visible text length (fast heuristic).
fn estimate_text_length(html: &str) -> usize {
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut text_len = 0;
    let mut tag_name = String::new();

    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            tag_name.clear();
        } else if c == '>' {
            in_tag = false;
            let tag_lower = tag_name.to_lowercase();
            if tag_lower == "script" {
                in_script = true;
            } else if tag_lower == "/script" {
                in_script = false;
            } else if tag_lower == "style" {
                in_style = true;
            } else if tag_lower == "/style" {
                in_style = false;
            }
        } else if in_tag {
            if tag_name.len() < 20 {
                tag_name.push(c);
            }
        } else if !in_script && !in_style && !c.is_whitespace() {
            text_len += 1;
        }
    }

    text_len
}

/// Estimate bytes within a tag type.
fn estimate_tag_bytes(html: &str, tag: &str) -> usize {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut total = 0;

    let html_lower = html.to_lowercase();
    let mut search_start = 0;

    while let Some(start) = html_lower[search_start..].find(&open) {
        let abs_start = search_start + start;
        if let Some(end_offset) = html_lower[abs_start..].find(&close) {
            let end = abs_start + end_offset + close.len();
            total += end - abs_start;
            search_start = end;
        } else {
            break;
        }
    }

    total
}

/// Estimate base64 encoded bytes.
fn estimate_base64_bytes(html: &str) -> usize {
    let mut total = 0;

    // Look for data: URIs
    let mut search_start = 0;
    while let Some(pos) = html[search_start..].find("data:") {
        let abs_pos = search_start + pos;
        // Find the end of the data URI
        if let Some(end) = html[abs_pos..].find(|c: char| c == '"' || c == '\'' || c == ')') {
            total += end;
        }
        search_start = abs_pos + 5;
    }

    total
}

/// Fast estimation of base64 bytes.
fn estimate_base64_bytes_fast(html: &[u8]) -> usize {
    static DATA_URI_MATCHER: LazyLock<AhoCorasick> = LazyLock::new(|| {
        AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .build(&["data:"])
            .expect("valid patterns")
    });
    // Count "data:" occurrences and estimate average size
    let count = DATA_URI_MATCHER.find_iter(html).count();
    count * 5_000 // Average data URI size estimate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_analysis_basic() {
        let html = r#"
            <html>
            <head><title>Test</title></head>
            <body>
                <p>This is some test content with enough text to be substantial for our analysis.</p>
                <p>More text here to ensure we have enough content for the analysis threshold.</p>
                <p>And even more text to make sure we pass the minimum text length threshold.</p>
                <p>Additional paragraph to ensure we have plenty of text content in this page.</p>
                <p>The goal is to have over 200 characters of visible text in this HTML document.</p>
            </body>
            </html>
        "#;

        let analysis = ContentAnalysis::analyze(html);

        assert!(!analysis.has_visual_elements);
        // With enough text content, no screenshot should be needed
        assert!(
            analysis.text_length >= 200,
            "Expected 200+ chars, got {}",
            analysis.text_length
        );
        assert!(!analysis.needs_screenshot);
    }

    #[test]
    fn test_content_analysis_with_iframe() {
        let html = r#"
            <html>
            <body>
                <iframe src="https://example.com"></iframe>
            </body>
            </html>
        "#;

        let analysis = ContentAnalysis::analyze(html);

        assert!(analysis.has_visual_elements);
        assert_eq!(analysis.iframe_count, 1);
        assert!(analysis.needs_screenshot);
    }

    #[test]
    fn test_content_analysis_spa() {
        let html = r#"
            <html>
            <body>
                <div id="root" data-reactroot></div>
                <script src="bundle.js"></script>
            </body>
            </html>
        "#;

        let analysis = ContentAnalysis::analyze(html);

        assert!(analysis.has_dynamic_content);
        assert!(analysis.is_thin_content);
    }

    #[test]
    fn test_quick_needs_screenshot() {
        assert!(ContentAnalysis::quick_needs_screenshot(
            "<iframe src='x'></iframe>"
        ));
        assert!(ContentAnalysis::quick_needs_screenshot(
            "<video src='x'></video>"
        ));
        assert!(ContentAnalysis::quick_needs_screenshot("<canvas></canvas>"));
        assert!(ContentAnalysis::quick_needs_screenshot("short"));

        let long_text = "a".repeat(2000);
        let html = format!("<html><body><p>{}</p></body></html>", long_text);
        assert!(!ContentAnalysis::quick_needs_screenshot(&html));
    }

    #[test]
    fn test_estimate_text_length() {
        let html = "<p>Hello World</p><script>console.log('ignored')</script>";
        let len = estimate_text_length(html);
        assert_eq!(len, 10); // "HelloWorld" without spaces
    }

    #[test]
    fn test_aho_corasick_visual_elements() {
        // Test Aho-Corasick matcher for visual elements
        assert!(ContentAnalysis::has_visual_elements_quick(
            "<IFRAME src='test'>"
        ));
        assert!(ContentAnalysis::has_visual_elements_quick("<Video>"));
        assert!(ContentAnalysis::has_visual_elements_quick("<CANVAS>"));
        assert!(ContentAnalysis::has_visual_elements_quick("<embed>"));
        assert!(ContentAnalysis::has_visual_elements_quick("<OBJECT>"));
        assert!(!ContentAnalysis::has_visual_elements_quick(
            "<div>No visuals</div>"
        ));
    }

    #[test]
    fn test_spa_detection() {
        // Test SPA indicator detection
        let react_html = r#"<div id="root" data-reactroot></div>"#;
        let analysis = ContentAnalysis::analyze(react_html);
        assert!(analysis.has_dynamic_content);

        let next_html = r#"<div id="__next"></div>"#;
        let analysis = ContentAnalysis::analyze(next_html);
        assert!(analysis.has_dynamic_content);

        let vue_html = r#"<div data-v-abc123></div>"#;
        let analysis = ContentAnalysis::analyze(vue_html);
        assert!(analysis.has_dynamic_content);

        let plain_html = r#"<div>Plain HTML</div>"#;
        let analysis = ContentAnalysis::analyze(plain_html);
        assert!(!analysis.has_dynamic_content);
    }

    #[test]
    fn test_content_analysis_summary() {
        let html = r#"<html><body><p>Test content here</p></body></html>"#;
        let analysis = ContentAnalysis::analyze(html);
        let summary = analysis.summary();
        assert!(summary.contains("text="));
        assert!(summary.contains("html="));
        assert!(summary.contains("ratio="));
    }
}
