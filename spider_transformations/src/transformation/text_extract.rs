use html2md::extended::sifter::WhitespaceSifter;
use lol_html::{element, html_content::TextType, text, RewriteStrSettings};

/// extract the text from HTML document.
pub fn extract_text(html: &str, custom: &Option<std::collections::HashSet<String>>) -> String {
    let mut extracted_text = String::new();

    let mut element_content_handlers = Vec::with_capacity(
        1 + custom
            .as_ref()
            .map_or(0, |c| if c.is_empty() { 0 } else { 1 }),
    );

    if let Some(ignore) = custom {
        let ignore_handler = element!(
            ignore.iter().cloned().collect::<Vec<String>>().join(","),
            |el| {
                el.remove();
                Ok(())
            }
        );

        element_content_handlers.push(ignore_handler);
    }

    element_content_handlers.push(text!(
        "*:not(script):not(style):not(svg):not(noscript):not(nav):not(footer)",
        |text| {
            if let TextType::RCData | TextType::Data = text.text_type() {
                let el_text = text.as_str().trim_start();
                if !el_text.is_empty() {
                    if !extracted_text.ends_with(' ') && !extracted_text.is_empty() {
                        extracted_text.push(' ');
                    }
                    extracted_text.push_str(el_text);
                }
                if text.text_type() == TextType::RCData {
                    extracted_text.push('\n');
                }
            }

            Ok(())
        }
    ));

    let _ = rewrite_str_empty(
        html,
        RewriteStrSettings {
            element_content_handlers,
            ..RewriteStrSettings::default()
        },
    );

    extracted_text.sift()
}

pub fn rewrite_str_empty<'h, 's, H: lol_html::HandlerTypes>(
    html: &str,
    settings: impl Into<lol_html::Settings<'h, 's, H>>,
) -> Result<(), lol_html::errors::RewritingError> {
    let mut rewriter = lol_html::HtmlRewriter::new(settings.into(), |_c: &[u8]| {});
    rewriter.write(html.as_bytes())?;
    rewriter.end()?;
    Ok(())
}
