use lol_html::{element, rewrite_str, text, RewriteStrSettings};

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
        "*:not(script):not(head):not(style):not(svg):not(noscript):not(nav):not(footer)",
        |text| {
            let el_text = text.as_str().trim_start();
            if !el_text.is_empty() {
                if !extracted_text.ends_with(' ') && !extracted_text.is_empty() {
                    extracted_text.push(' ');
                }
                extracted_text.push_str(el_text);
            }
            Ok(())
        }
    ));

    let _ = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers,
            ..RewriteStrSettings::default()
        },
    );

    extracted_text
}
