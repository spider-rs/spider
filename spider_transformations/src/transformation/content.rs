use crate::html2xml::convert_html_to_xml;
use aho_corasick::AhoCorasick;
use html2md;
use regex::Regex;
use serde::{Deserialize, Deserializer};
use spider::lazy_static::lazy_static;
use spider::packages::scraper::Html;
use spider::packages::scraper::{ElementRef, Selector};
use spider::page::Page;
use spider::url::Url;
use spider::utils::clean_html;
use std::collections::HashMap;

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

    pub static ref SELECTOR: std::sync::Arc<Selector> = unsafe {
        Selector::parse(&r##"body"##)
            .unwrap_unchecked()
            .into()
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
}

/// ignore tags for markdown transformation
#[derive(Clone)]
pub struct IgnoreTagFactory;

impl html2md::TagHandlerFactory for IgnoreTagFactory {
    fn instantiate(&self) -> Box<dyn html2md::TagHandler> {
        Box::new(self.clone())
    }
}

impl html2md::TagHandler for IgnoreTagFactory {
    fn handle(&mut self, _tag: &html2md::Handle, _printer: &mut html2md::StructuredPrinter) {}
    fn after_handle(&mut self, _printer: &mut html2md::StructuredPrinter) {}
    fn skip_descendants(&self) -> bool {
        true
    }
}

/// is the content html and safe for formatting.
pub fn is_html_content(bytes: &[u8], url: &spider::url::Url) -> bool {
    // Check for common HTML tags in the byte content
    const HTML_TAGS: [&[u8]; 4] = [b"<html", b"<!doctype html", b"<head", b"<body"];

    // Check the beginning of the byte content for HTML tags or doctype
    if bytes.len() >= 1024 {
        for tag in &HTML_TAGS {
            if bytes[..1024]
                .windows(tag.len())
                .any(|window| window == *tag)
            {
                return true;
            }
        }
    } else {
        for tag in &HTML_TAGS {
            if bytes.windows(tag.len()).any(|window| window == *tag) {
                return true;
            }
        }
    }

    // Perform some heuristic checks on the URL in case it's not apparent from content

    if let Some(extension) = url.path_segments().and_then(|segments| segments.last()) {
        if extension.ends_with(".html") || extension.ends_with(".htm") {
            return true;
        }
    }

    // Check for MIME type if needed. This can be done via HTTP headers if available in a broader context.

    false
}

/// clean the markdown with aho. This does a triple pass across the content.
pub fn aho_clean_markdown(html: &str) -> String {
    // handle the error on replace all
    // if the content is small just use an aho replacement
    if html.len() <= 40 {
        let base_clean = match AHO.try_replace_all(&html, &*AHO_REPLACEMENTS) {
            Ok(r) => r,
            _ => html.into(),
        };
        base_clean
    } else {
        // regex smooth clean multiple
        let cleaned_html = CLEAN_MARKDOWN_REGEX.replace_all(&html, |caps: &regex::Captures| {
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

/// transform the content to markdown
pub fn transform_markdown(html: &str, commonmark: bool) -> String {
    let mut tag_factory: HashMap<String, Box<dyn html2md::TagHandlerFactory>> = HashMap::new();
    let tag = Box::new(IgnoreTagFactory {});

    tag_factory.insert(String::from("script"), tag.clone());
    tag_factory.insert(String::from("style"), tag.clone());
    tag_factory.insert(String::from("noscript"), tag.clone());

    if !commonmark {
        tag_factory.insert(String::from("meta"), tag.clone());
    }

    tag_factory.insert(String::from("iframe"), tag);

    let html = html2md::parse_html_custom(&html, &tag_factory, commonmark);
    let html = aho_clean_markdown(&html);
    html
}

/// transform the content to text raw
pub fn transform_text(html: &str) -> String {
    let fragment = Html::parse_document(&html);
    let d = fragment
        .select(SELECTOR.as_ref())
        .filter_map(|c| ElementRef::wrap(*c))
        .collect::<Vec<_>>();
    super::text_extract::extract_text(&d)
}

/// get the HTML content for the page.
fn get_html(res: &Page, encoding: &Option<String>) -> String {
    match encoding {
        Some(ref encoding) => res.get_html_encoded(encoding),
        _ => res.get_html(),
    }
}

/// Transform format the content.
pub fn transform_content(
    res: &Page,
    c: &TransformConfig,
    encoding: &Option<String>,
    root_selector: &Option<String>,
) -> String {
    let return_format = c.return_format;
    let filter_images = c.filter_images;
    let url_parsed = res.get_url_parsed().as_ref();

    match return_format {
        ReturnFormat::Raw | ReturnFormat::Bytes => {
            if c.readability {
                match llm_readability::extractor::extract(
                    &mut res.get_html_bytes_u8(),
                    match url_parsed {
                        Some(u) => u,
                        _ => &EXAMPLE_URL,
                    },
                    &None,
                ) {
                    Ok(product) => product.content,
                    _ => get_html(res, &encoding),
                }
            } else {
                get_html(res, &encoding)
            }
        }
        ReturnFormat::CommonMark => {
            let mut html = if c.readability && !res.is_empty() {
                match llm_readability::extractor::extract(
                    &mut res.get_html_bytes_u8(),
                    match url_parsed {
                        Some(u) => u,
                        _ => &EXAMPLE_URL,
                    },
                    &None,
                ) {
                    Ok(product) => {
                        if product.content.is_empty() {
                            get_html(res, &encoding)
                        } else {
                            product.content
                        }
                    }
                    _ => get_html(res, &encoding),
                }
            } else {
                get_html(res, &encoding)
            };

            let mut tag_factory: HashMap<String, Box<dyn html2md::TagHandlerFactory>> =
                HashMap::new();
            let tag = Box::new(IgnoreTagFactory {});

            tag_factory.insert(String::from("script"), tag.clone());
            tag_factory.insert(String::from("style"), tag.clone());
            tag_factory.insert(String::from("noscript"), tag.clone());

            if filter_images {
                tag_factory.insert(String::from("img"), tag.clone());
                tag_factory.insert(String::from("picture"), tag.clone());
                html = clean_html(&html)
            }

            tag_factory.insert(String::from("iframe"), tag);

            let html = html2md::parse_html_custom(&html, &tag_factory, true);
            let html = aho_clean_markdown(&html);

            html
        }
        ReturnFormat::Markdown => {
            let mut html = if c.readability {
                match llm_readability::extractor::extract(
                    &mut res.get_html_bytes_u8(),
                    match url_parsed {
                        Some(u) => u,
                        _ => &EXAMPLE_URL,
                    },
                    &None,
                ) {
                    Ok(product) => {
                        if product.content.is_empty() {
                            get_html(res, encoding)
                        } else {
                            product.content
                        }
                    }
                    _ => get_html(res, encoding),
                }
            } else {
                get_html(res, encoding)
            };

            let mut tag_factory: HashMap<String, Box<dyn html2md::TagHandlerFactory>> =
                HashMap::new();

            let tag = Box::new(IgnoreTagFactory {});

            tag_factory.insert(String::from("script"), tag.clone());
            tag_factory.insert(String::from("style"), tag.clone());
            tag_factory.insert(String::from("noscript"), tag.clone());

            if filter_images {
                tag_factory.insert(String::from("img"), tag.clone());
                tag_factory.insert(String::from("picture"), tag.clone());
                html = clean_html(&html)
            }

            tag_factory.insert(String::from("iframe"), tag);

            let html = html2md::parse_html_custom(&html, &tag_factory, false);
            let html = aho_clean_markdown(&html);
            html
        }
        ReturnFormat::Html2Text => match encoding {
            Some(ref e) => {
                let b = res.get_html_encoded(e);
                let b = if c.readability {
                    match llm_readability::extractor::extract(
                        &mut b.as_bytes(),
                        match res.get_url_parsed() {
                            Some(u) => u,
                            _ => &EXAMPLE_URL,
                        },
                        &None,
                    ) {
                        Ok(product) => {
                            if product.content.is_empty() {
                                get_html(res, &encoding)
                            } else {
                                product.content
                            }
                        }
                        _ => b,
                    }
                } else {
                    b
                };

                if b.len() > 0 {
                    crate::html2text::from_read(&b.as_bytes()[..], b.len())
                } else {
                    Default::default()
                }
            }
            _ => {
                if c.readability {
                    match llm_readability::extractor::extract(
                        &mut res.get_html_bytes_u8(),
                        match url_parsed {
                            Some(u) => u,
                            _ => &EXAMPLE_URL,
                        },
                        &None,
                    ) {
                        Ok(product) => {
                            let b = {
                                if product.content.is_empty() {
                                    res.get_html_bytes_u8()
                                } else {
                                    product.content.as_bytes()
                                }
                            };

                            if b.len() > 0 {
                                crate::html2text::from_read(&b[..], b.len())
                            } else {
                                Default::default()
                            }
                        }
                        _ => match res.get_bytes() {
                            Some(b) => {
                                if b.len() > 0 {
                                    crate::html2text::from_read(&b[..], b.len())
                                } else {
                                    Default::default()
                                }
                            }
                            _ => Default::default(),
                        },
                    }
                } else {
                    match res.get_bytes() {
                        Some(b) => {
                            if b.len() > 0 {
                                crate::html2text::from_read(&b[..], b.len())
                            } else {
                                Default::default()
                            }
                        }
                        _ => Default::default(),
                    }
                }
            }
        },
        ReturnFormat::Text => {
            let b = if c.readability {
                match llm_readability::extractor::extract(
                    &mut res.get_html_bytes_u8(),
                    match url_parsed {
                        Some(u) => u,
                        _ => &EXAMPLE_URL,
                    },
                    &None,
                ) {
                    Ok(product) => {
                        if product.content.is_empty() {
                            get_html(res, encoding)
                        } else {
                            product.content
                        }
                    }
                    _ => get_html(res, encoding),
                }
            } else {
                get_html(res, encoding)
            };
            let fragment = Html::parse_document(&b);

            let d = if root_selector.is_some() {
                let selector = &match root_selector {
                    Some(ref root_selector) => match Selector::parse(root_selector) {
                        Ok(qs) => qs,
                        _ => SELECTOR.as_ref().clone(),
                    },
                    _ => SELECTOR.as_ref().clone(),
                };
                fragment
                    .select(&selector)
                    .filter_map(|c| ElementRef::wrap(*c))
                    .collect::<Vec<_>>()
            } else {
                fragment
                    .select(SELECTOR.as_ref())
                    .filter_map(|c| ElementRef::wrap(*c))
                    .collect::<Vec<_>>()
            };

            super::text_extract::extract_text(&d)
        }
        ReturnFormat::XML => {
            let target_url = match url_parsed {
                Some(u) => u.to_string(),
                _ => EXAMPLE_URL.to_string(),
            };

            if c.readability {
                match llm_readability::extractor::extract(
                    &mut res.get_html_bytes_u8(),
                    match url_parsed {
                        Some(u) => u,
                        _ => &EXAMPLE_URL,
                    },
                    &None,
                ) {
                    Ok(product) => {
                        if let Ok(xml) =
                            convert_html_to_xml(&product.content, &target_url, &encoding)
                        {
                            xml
                        } else {
                            Default::default()
                        }
                    }
                    _ => {
                        if let Ok(xml) =
                            convert_html_to_xml(&get_html(res, &encoding), &target_url, &encoding)
                        {
                            xml
                        } else {
                            Default::default()
                        }
                    }
                }
            } else {
                if let Ok(xml) =
                    convert_html_to_xml(&get_html(res, &encoding), &target_url, &encoding)
                {
                    xml
                } else {
                    Default::default()
                }
            }
        }
    }
}
