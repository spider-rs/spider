use lol_html::{rewrite_str, text, RewriteStrSettings};

/// extract the text from HTML document.
pub fn extract_text(html: &str) -> String {
    let mut extracted_text = String::new();

    let _ = rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: vec![text!("*", |text| {
                let el_text = text.as_str();
                if !el_text.is_empty() {
                    if !extracted_text.ends_with(' ') && !extracted_text.is_empty() {
                        extracted_text.push(' ');
                    }
                    extracted_text.push_str(el_text);
                }
                Ok(())
            })],
            ..RewriteStrSettings::default()
        },
    );

    extracted_text
}
