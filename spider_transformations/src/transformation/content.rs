use crate::html2xml::convert_html_to_xml;
use aho_corasick::AhoCorasick;
use html2md;
use phf::phf_set;
use regex::Regex;
use serde::{Deserialize, Deserializer};
use spider::auto_encoder::is_binary_file;
use spider::lazy_static::lazy_static;
use spider::page::Page;
use spider::url::Url;
use spider::utils::clean_html;

lazy_static! {
    static ref AHO: AhoCorasick = AhoCorasick::new(["\n\n\n", "\n  \n  ", "\n\n\n\n\n"]).unwrap();
    static ref AHO_REPLACEMENTS: [&'static str; 3] = [
        "\n\n",  // Replace triple newlines with two newlines
        "\n\n",  // Replace multiple spaces with two newlines
        "\n\n",  // Replace five newlines with two newlines
    ];
    static ref CLEAN_MARKDOWN_REGEX: Regex =  {
        Regex::new(
            r"(?m)^[ \t]+|[ \t]+$|[ \t]+|\s*\n\s*\n\s*"
        ).unwrap()

    };
    static ref EXAMPLE_URL: Url = Url::parse("https://example.net").expect("invalid url");
}

/// The return format for the content.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReturnFormat {
    #[default]
    /// Default format
    Raw,
    /// Bytes - this does not change the output type and more aligned for what the input is.
    Bytes,
    /// Text
    Text,
    /// Text Mapping
    Html2Text,
    /// Markdown
    Markdown,
    /// Commonmark
    CommonMark,
    /// XML
    XML,
}

impl ReturnFormat {
    /// Convert the content from string match
    pub fn from_str(s: &str) -> ReturnFormat {
        match s {
            "text" | "Text" | "TEXT" => ReturnFormat::Text,
            "html2text" | "Html2text" | "HTML2TEXT" | "html_2_text" | "HTML_2_TEXT" => {
                ReturnFormat::Html2Text
            }
            "markdown" | "Markdown" | "MARKDOWN" => ReturnFormat::Markdown,
            "raw" | "RAW" | "Raw" => ReturnFormat::Raw,
            "bytes" | "Bytes" | "BYTES" => ReturnFormat::Bytes,
            "commonmark" | "CommonMark" | "COMMONMARK" => ReturnFormat::CommonMark,
            "xml" | "XML" | "XmL" | "Xml" => ReturnFormat::XML,
            _ => ReturnFormat::Raw,
        }
    }
}

impl<'de> Deserialize<'de> for ReturnFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        match s.as_ref() {
            "text" | "Text" | "TEXT" => Ok(ReturnFormat::Text),
            "html2text" | "Html2text" | "HTML2TEXT" | "html_2_text" | "HTML_2_TEXT" => {
                Ok(ReturnFormat::Html2Text)
            }
            "markdown" | "Markdown" | "MARKDOWN" => Ok(ReturnFormat::Markdown),
            "raw" | "RAW" | "Raw" => Ok(ReturnFormat::Raw),
            "bytes" | "Bytes" | "BYTES" => Ok(ReturnFormat::Bytes),
            "commonmark" | "CommonMark" | "COMMONMARK" => Ok(ReturnFormat::CommonMark),
            "xml" | "XML" | "XmL" | "Xml" => Ok(ReturnFormat::XML),
            _ => Ok(ReturnFormat::Raw),
        }
    }
}

/// Transformation configuration adjustments.
#[derive(Debug, Default, Clone, Copy)]
pub struct TransformConfig {
    /// Readability mode.
    pub readability: bool,
    /// The return format to use.
    pub return_format: ReturnFormat,
    /// Filter Images.
    pub filter_images: bool,
    /// Trim the content for LLMs.
    pub clean_html: bool,
    /// Filter svgs.
    pub filter_svg: bool,
    /// Main content for the page. Exclude the nav, footer, and etc.
    pub main_content: bool,
}

/// Select elements to show or hide using a CSS selector.
#[derive(Debug, Default, Clone)]
pub struct SelectorConfiguration {
    /// The root html selector.
    pub root_selector: Option<String>,
    /// Exclude the matching css selector from the output.
    pub exclude_selector: Option<String>,
}

/// is the content html and safe for formatting.
static HTML_TAGS: phf::Set<&'static [u8]> = phf_set! {
    b"<!doctype html",
    b"<html",
    b"<document",
};

/// valid file extensions that will render html from a program
pub static VALID_EXTENSIONS: phf::Set<&'static str> = phf_set! {
    ".html",
    ".htm",
    ".shtml",
    ".asp",
    ".aspx",
    ".php",
    ".jps",
    ".jpsx",
    ".jsp",
    ".cfm",
    ".xhtml",
    ".rhtml",
    ".phtml",
    ".erb",
};

/// Check if the content is HTML.
pub fn is_html_content(bytes: &[u8], url: &Url) -> bool {
    let check_bytes = if bytes.len() > 1024 {
        &bytes[..1024]
    } else {
        bytes
    };

    for tag in HTML_TAGS.iter() {
        if check_bytes
            .windows(tag.len())
            .any(|window| window.eq_ignore_ascii_case(tag))
        {
            return true;
        }
    }

    // Heuristic check on URL extension
    if let Some(extension) = url
        .path_segments()
        .and_then(|segments| segments.last().and_then(|s| s.split('.').last()))
    {
        if VALID_EXTENSIONS.contains(extension) {
            return true;
        }
    }
    false
}

/// clean the markdown with aho. This does a triple pass across the content.
pub fn aho_clean_markdown(html: &str) -> String {
    // handle the error on replace all
    // if the content is small just use an aho replacement
    if html.len() <= 40 {
        match AHO.try_replace_all(html, &*AHO_REPLACEMENTS) {
            Ok(r) => r,
            _ => html.into(),
        }
    } else {
        // regex smooth clean multiple
        let cleaned_html = CLEAN_MARKDOWN_REGEX.replace_all(html, |caps: &regex::Captures| {
            let matched = match caps.get(0) {
                Some(m) => m.as_str(),
                _ => Default::default(),
            };
            if matched.contains('\n') && matched.chars().filter(|&c| c == '\n').count() >= 3 {
                "\n\n"
            } else if matched.contains('\n') {
                "\n"
            } else {
                " "
            }
        });

        cleaned_html.into()
    }
}

/// Clean the html elements from the markup.
pub fn clean_html_elements(html: &str, tags: Vec<&str>) -> String {
    use lol_html::{element, rewrite_str, RewriteStrSettings};
    match rewrite_str(
        html,
        RewriteStrSettings {
            element_content_handlers: tags
                .iter()
                .map(|tag| {
                    element!(tag, |el| {
                        el.remove();
                        Ok(())
                    })
                })
                .collect::<Vec<_>>(),
            ..RewriteStrSettings::default()
        },
    ) {
        Ok(r) => r,
        _ => html.into(),
    }
}

/// Buld the static ignore list of html elements.
pub(crate) fn build_static_vector(config: &TransformConfig) -> Vec<&'static str> {
    let mut tags = Vec::new();

    if config.filter_images {
        tags.push("img");
        tags.push("picture");
    }

    if config.filter_svg {
        tags.push("svg");
    }

    if config.main_content {
        tags.push("nav");
        tags.push("footer");
        tags.push("aside");
    }

    tags
}

/// transform the content to markdown shortcut
pub fn transform_markdown(html: &str, commonmark: bool) -> String {
    html2md::rewrite_html_custom_with_url(html, &None, commonmark, &None)
}

/// transform the content to markdown shortcut send
pub async fn transform_markdown_send(html: &str, commonmark: bool) -> String {
    html2md::rewrite_html_custom_with_url_streaming(html, &None, commonmark, &None).await
}

/// transform the content to text raw shortcut
pub fn transform_text(html: &str) -> String {
    super::text_extract::extract_text(html, &Default::default())
}

/// transform the content to text raw shortcut and custom ignore
pub fn transform_text_ignore(
    html: &str,
    custom_ignore: &Option<std::collections::HashSet<String>>,
) -> String {
    super::text_extract::extract_text(html, custom_ignore)
}

/// get the HTML content for the page.
fn get_html(res: &Page, encoding: &Option<String>) -> String {
    match encoding {
        Some(ref encoding) => res.get_html_encoded(encoding),
        _ => res.get_html(),
    }
}

/// get the html with the root selector
fn get_html_with_selector(
    res: &Page,
    encoding: &Option<String>,
    selector_config: &Option<SelectorConfiguration>,
) -> String {
    use scraper::{Html, Selector};
    let html = get_html(res, encoding);

    if let Some(selector_config) = selector_config.as_ref() {
        let mut fragment = Html::parse_fragment(&html);

        if let Some(selector) = selector_config.root_selector.as_ref() {
            if let Ok(parsed_selector) = Selector::parse(selector) {
                if let Some(root_node) = fragment.select(&parsed_selector).next() {
                    if selector_config.exclude_selector.is_some() {
                        fragment.clone_from(&Html::parse_fragment(&root_node.html()));
                    } else {
                        // return the direct html found
                        return root_node.html();
                    }
                }
            }
        }

        if let Some(exclude_selector) = selector_config.exclude_selector.as_ref() {
            if let Ok(exclude_sel) = Selector::parse(exclude_selector) {
                let mut elements_to_remove = vec![];

                for elem in fragment.root_element().select(&exclude_sel) {
                    elements_to_remove.push(elem.id());
                }

                for id in elements_to_remove {
                    fragment.remove_node(id);
                }
            }
        }

        return fragment.root_element().html();
    }

    html
}

/// Transform format the content.
pub fn transform_content(
    res: &Page,
    c: &TransformConfig,
    encoding: &Option<String>,
    selector_config: &Option<SelectorConfiguration>,
    ignore_tags: &Option<Vec<String>>,
) -> String {
    let base_html = get_html_with_selector(res, encoding, selector_config);

    // prevent transforming binary files or re-encoding it
    if is_binary_file(res.get_html_bytes_u8()) {
        return base_html;
    }

    let url_parsed = res.get_url_parsed_ref();

    let base_html = {
        let mut ignore_list = build_static_vector(c);

        if let Some(ignore) = ignore_tags {
            ignore_list.extend(ignore.iter().map(|s| s.as_str()));
        }

        if ignore_list.is_empty() {
            base_html
        } else {
            clean_html_elements(&base_html, ignore_list)
        }
    };

    // process readability
    let base_html = if c.readability {
        match llm_readability::extractor::extract(
            &mut base_html.as_bytes(),
            match url_parsed {
                Some(u) => u,
                _ => &EXAMPLE_URL,
            },
        ) {
            Ok(product) => product.content,
            _ => base_html,
        }
    } else {
        base_html
    };

    let base_html = if c.clean_html {
        clean_html(&base_html)
    } else {
        base_html
    };

    let mut tag_factory = None;

    if let Some(ignore) = ignore_tags {
        let mut tag_factor = std::collections::HashSet::with_capacity(ignore.len());
        for ignore_tag_name in ignore {
            tag_factor.insert(ignore_tag_name.into());
        }
        tag_factory.replace(tag_factor);
    }

    match c.return_format {
        ReturnFormat::Raw | ReturnFormat::Bytes => base_html,
        ReturnFormat::CommonMark => {
            html2md::rewrite_html_custom_with_url(&base_html, &tag_factory, true, url_parsed)
        }
        ReturnFormat::Markdown => {
            html2md::rewrite_html_custom_with_url(&base_html, &tag_factory, false, url_parsed)
        }
        ReturnFormat::Html2Text => {
            if !base_html.is_empty() {
                crate::html2text::from_read(base_html.as_bytes(), base_html.len())
            } else {
                base_html
            }
        }
        ReturnFormat::Text => super::text_extract::extract_text(&base_html, &tag_factory),
        ReturnFormat::XML => convert_html_to_xml(
            base_html.trim(),
            &match url_parsed {
                Some(u) => u.to_string(),
                _ => EXAMPLE_URL.to_string(),
            },
            encoding,
        )
        .unwrap_or_default(),
    }
}

/// Transform format the content send.
pub async fn transform_content_send(
    res: &Page,
    c: &TransformConfig,
    encoding: &Option<String>,
    selector_config: &Option<SelectorConfiguration>,
    ignore_tags: &Option<Vec<String>>,
) -> String {
    let base_html = get_html_with_selector(res, encoding, selector_config);

    // prevent transforming binary files or re-encoding it
    if is_binary_file(res.get_html_bytes_u8()) {
        return base_html;
    }

    let url_parsed = res.get_url_parsed_ref();

    let base_html = {
        let mut ignore_list = build_static_vector(c);

        if let Some(ignore) = ignore_tags {
            ignore_list.extend(ignore.iter().map(|s| s.as_str()));
        }

        if ignore_list.is_empty() {
            base_html
        } else {
            clean_html_elements(&base_html, ignore_list)
        }
    };

    // process readability
    let base_html = if c.readability {
        match llm_readability::extractor::extract(
            &mut base_html.as_bytes(),
            match url_parsed {
                Some(u) => u,
                _ => &EXAMPLE_URL,
            },
        ) {
            Ok(product) => product.content,
            _ => base_html,
        }
    } else {
        base_html
    };

    let base_html = if c.clean_html {
        clean_html(&base_html)
    } else {
        base_html
    };

    let mut tag_factory = None;

    if let Some(ignore) = ignore_tags {
        let mut tag_factor = std::collections::HashSet::with_capacity(ignore.len());
        for ignore_tag_name in ignore {
            tag_factor.insert(ignore_tag_name.into());
        }
        tag_factory.replace(tag_factor);
    }

    match c.return_format {
        ReturnFormat::Raw | ReturnFormat::Bytes => base_html,
        ReturnFormat::CommonMark => {
            html2md::rewrite_html_custom_with_url_streaming(
                &base_html,
                &tag_factory,
                true,
                url_parsed,
            )
            .await
        }
        ReturnFormat::Markdown => {
            html2md::rewrite_html_custom_with_url_streaming(
                &base_html,
                &tag_factory,
                false,
                url_parsed,
            )
            .await
        }
        ReturnFormat::Html2Text => {
            if !base_html.is_empty() {
                crate::html2text::from_read(base_html.as_bytes(), base_html.len())
            } else {
                base_html
            }
        }
        ReturnFormat::Text => {
            super::text_extract::extract_text_streaming(&base_html, &tag_factory).await
        }
        ReturnFormat::XML => convert_html_to_xml(
            base_html.trim(),
            &match url_parsed {
                Some(u) => u.to_string(),
                _ => EXAMPLE_URL.to_string(),
            },
            encoding,
        )
        .unwrap_or_default(),
    }
}

/// transform the content to bytes to prevent loss of precision.
pub fn transform_content_to_bytes(
    res: &Page,
    c: &TransformConfig,
    encoding: &Option<String>,
    selector_config: &Option<SelectorConfiguration>,
    ignore_tags: &Option<Vec<String>>,
) -> Vec<u8> {
    if is_binary_file(res.get_html_bytes_u8()) {
        let b = res.get_bytes();
        if let Some(b) = b {
            b.clone()
        } else {
            Default::default()
        }
    } else {
        transform_content(res, c, encoding, selector_config, ignore_tags).into()
    }
}
