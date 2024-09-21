use html5ever::driver::ParseOpts;
use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
pub use markup5ever_rcdom::{Handle, NodeData, RcDom};
use regex::Regex;
use spider::lazy_static::lazy_static;
use std::boxed::Box;
use std::collections::HashMap;
pub mod anchors;
pub mod codes;
pub mod common;
pub mod containers;
pub mod dummy;
pub mod headers;
pub mod iframes;
pub mod images;
pub mod lists;
pub mod paragraphs;
pub mod quotes;
pub mod styles;
pub mod tables;

use anchors::AnchorHandler;
use codes::CodeHandler;
use containers::ContainerHandler;
use dummy::DummyHandler;
use dummy::HtmlCherryPickHandler;
use dummy::IdentityHandler;
use headers::HeaderHandler;
use iframes::IframeHandler;
use images::ImgHandler;
use lists::ListHandler;
use lists::ListItemHandler;
use paragraphs::ParagraphHandler;
use quotes::QuoteHandler;
use styles::StyleHandler;
use tables::TableHandler;

lazy_static! {
    static ref EXCESSIVE_WHITESPACE_PATTERN: Regex = Regex::new("\\s{2,}").expect("valid regex pattern");   // for HTML on-the-fly cleanup
    static ref EMPTY_LINE_PATTERN: Regex = Regex::new("(?m)^ +$").expect("valid regex pattern");            // for Markdown post-processing
    static ref EXCESSIVE_NEWLINE_PATTERN: Regex = Regex::new("\\n{3,}").expect("valid regex pattern");      // for Markdown post-processing
    static ref TRAILING_SPACE_PATTERN: Regex = Regex::new("(?m)(\\S) $").expect("valid regex pattern");     // for Markdown post-processing
    static ref LEADING_NEWLINES_PATTERN: Regex = Regex::new("^\\n+").expect("valid regex pattern");         // for Markdown post-processing
    static ref LAST_WHITESPACE_PATTERN: Regex = Regex::new("\\s+$").expect("valid regex pattern");          // for Markdown post-processing
    static ref START_OF_LINE_PATTERN: Regex = Regex::new("(^|\\n) *$").expect("valid regex pattern");                  // for Markdown escaping
    static ref MARKDOWN_STARTONLY_KEYCHARS: Regex = Regex::new(r"^(\s*)([=>+\-#])").expect("valid regex pattern");     // for Markdown escaping
    static ref MARKDOWN_MIDDLE_KEYCHARS: Regex = Regex::new(r"[<>*\\_~]").expect("valid regex pattern");               // for Markdown escaping
    static ref CLEANUP_PATTERN: Regex = Regex::new(
        r"(?x)
        (?P<empty_line>^\s*$\n)|
        (?P<excessive_newlines>\n{3,})|
        (?P<trailing_space>(\S) )|
        (?P<leading_newlines>^\n+)|
        (?P<last_whitespace>\s+$)|
        (?P<empty_image>!\[\]\(\))|
        (?P<leading_whitespace>^\s+)"
    ).expect("valid regex pattern");
}

/// Custom variant of main function. Allows to pass custom tag<->tag factory pairs
/// in order to register custom tag hadler for tags you want.
///
/// You can also override standard tag handlers this way
/// # Arguments
/// `html` is source HTML as `String`
/// `custom` is custom tag hadler producers for tags you want, can be empty
pub fn parse_html_custom(
    html: &str,
    custom: &HashMap<String, Box<dyn TagHandlerFactory>>,
    commonmark: bool,
) -> String {
    match parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
    {
        Ok(dom) => {
            let mut result = StructuredPrinter::default();
            walk(&dom.document, &mut result, custom, commonmark);
            clean_markdown(&result.data)
        }
        _ => Default::default(),
    }
}

/// Main function of this library. Parses incoming HTML, converts it into Markdown
/// and returns converted string.
/// # Arguments
/// `html` is source HTML as `String`
pub fn parse_html(html: &str, commonmark: bool) -> String {
    parse_html_custom(html, &HashMap::default(), commonmark)
}

/// Same as `parse_html` but retains all "span" html elements intact
/// Markdown parsers usually strip them down when rendering but they
/// may be useful for later processing
pub fn parse_html_extended(html: &str, commonmark: bool) -> String {
    struct SpanAsIsTagFactory;

    impl TagHandlerFactory for SpanAsIsTagFactory {
        fn instantiate(&self) -> Box<dyn TagHandler> {
            Box::new(HtmlCherryPickHandler::default())
        }
    }

    let mut tag_factory: HashMap<String, Box<dyn TagHandlerFactory>> = HashMap::new();
    tag_factory.insert(String::from("span"), Box::new(SpanAsIsTagFactory {}));
    parse_html_custom(html, &tag_factory, commonmark)
}

/// Recursively walk through all DOM tree and handle all elements according to
/// HTML tag -> Markdown syntax mapping. Text content is trimmed to one whitespace according to HTML5 rules.
///
/// # Arguments
/// `input` is DOM tree or its subtree
/// `result` is output holder with position and context tracking
/// `custom` is custom tag hadler producers for tags you want, can be empty
fn walk(
    input: &Handle,
    result: &mut StructuredPrinter,
    custom: &HashMap<String, Box<dyn TagHandlerFactory>>,
    commonmark: bool,
) {
    let mut handler: Box<dyn TagHandler> = Box::new(DummyHandler::default());
    let mut tag_name = String::default();

    match input.data {
        NodeData::Document | NodeData::Doctype { .. } | NodeData::ProcessingInstruction { .. } => {}
        NodeData::Text { ref contents } => {
            let mut text = contents.borrow().to_string();
            let inside_pre = result.parent_chain.iter().any(|tag| tag == "pre");
            if inside_pre {
                // this is preformatted text, insert as-is
                result.append_str(&text);
            } else if !(text.trim().len() == 0
                && (result.data.chars().last() == Some('\n')
                    || result.data.chars().last() == Some(' ')))
            {
                // in case it's not just a whitespace after the newline or another whitespace

                // regular text, collapse whitespace and newlines in text
                let inside_code = result.parent_chain.iter().any(|tag| tag == "code");
                if !inside_code {
                    text = escape_markdown(result, &text);
                }
                let minified_text = EXCESSIVE_WHITESPACE_PATTERN.replace_all(&text, " ");
                let minified_text = minified_text.trim_matches(|ch: char| ch == '\n' || ch == '\r');
                result.append_str(&minified_text);
            }
        }
        NodeData::Comment { .. } => {} // ignore comments
        NodeData::Element { ref name, .. } => {
            tag_name = name.local.to_string();
            let inside_pre = result.parent_chain.iter().any(|tag| tag == "pre");

            if inside_pre {
                // don't add any html tags inside the pre section
                handler = Box::new(DummyHandler::default());
            } else if custom.contains_key(&tag_name) {
                match custom.get(&tag_name) {
                    Some(factory) => {
                        // have user-supplied factory, instantiate a handler for this tag
                        handler = factory.instantiate();
                    }
                    _ => (),
                }
            } else {
                // no user-supplied factory, take one of built-in ones
                handler = match tag_name.as_ref() {
                    // containers
                    "div" | "section" | "header" | "footer" => {
                        Box::new(ContainerHandler::default())
                    }
                    // pagination, breaks
                    "p" | "br" | "hr" => Box::new(ParagraphHandler::default()),
                    "q" | "cite" | "blockquote" => Box::new(QuoteHandler::default()),
                    // spoiler tag
                    "details" | "summary" => Box::new(HtmlCherryPickHandler::new(commonmark)),
                    // formatting
                    "b" | "i" | "s" | "strong" | "em" | "del" => Box::new(StyleHandler::default()),
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Box::new(HeaderHandler::default()),
                    "pre" | "code" => Box::new(CodeHandler::default()),
                    // images, links
                    "img" => Box::new(ImgHandler::new(commonmark)),
                    "a" => Box::new(AnchorHandler::default()),
                    // lists
                    "ol" | "ul" | "menu" => Box::new(ListHandler::default()),
                    "li" => Box::new(ListItemHandler::default()),
                    // as-is
                    "sub" | "sup" => Box::new(IdentityHandler::default()),
                    // tables, handled fully internally as markdown can't have nested content in tables
                    // supports only single tables as of now
                    "table" => Box::new(TableHandler::default()),
                    "iframe" => Box::new(IframeHandler::default()),
                    // other
                    "html" | "head" | "body" => Box::new(DummyHandler::default()),
                    _ => Box::new(DummyHandler::default()),
                };
            }
        }
    }

    // handle this tag, while it's not in parent chain
    // and doesn't have child siblings
    handler.handle(&input, result);

    // save this tag name as parent for child nodes
    result.parent_chain.push(tag_name.to_string()); // e.g. it was ["body"] and now it's ["body", "p"]
    let current_depth = result.parent_chain.len(); // e.g. it was 1 and now it's 2

    // create space for siblings of next level
    result.siblings.insert(current_depth, vec![]);

    for child in input.children.borrow().iter() {
        if handler.skip_descendants() {
            continue;
        }

        walk(&child, result, custom, commonmark);

        match child.data {
            NodeData::Element { ref name, .. } => match result.siblings.get_mut(&current_depth) {
                Some(el) => el.push(name.local.to_string()),
                _ => (),
            },
            _ => (),
        };
    }

    // clear siblings of next level
    result.siblings.remove(&current_depth);

    // release parent tag
    result.parent_chain.pop();

    // finish handling of tag - parent chain now doesn't contain this tag itself again
    handler.after_handle(result);
}

/// This conversion should only be applied to text tags
///
/// Escapes text inside HTML tags so it won't be recognized as Markdown control sequence
/// like list start or bold text style
fn escape_markdown(result: &StructuredPrinter, text: &str) -> String {
    // always escape bold/italic/strikethrough
    let data: std::borrow::Cow<str> = MARKDOWN_MIDDLE_KEYCHARS.replace_all(&text, "\\$0");

    // if we're at the start of the line we need to escape list- and quote-starting sequences
    let data = if START_OF_LINE_PATTERN.is_match(&result.data) {
        MARKDOWN_STARTONLY_KEYCHARS.replace(&data, "$1\\$2")
    } else {
        data
    };

    // no handling of more complicated cases such as
    // ![] or []() ones, for now this will suffice
    data.into()
}

/// Called after all processing has been finished
///
/// Clears excessive punctuation that would be trimmed by renderer anyway
fn clean_markdown(text: &str) -> String {
    CLEANUP_PATTERN
        .replace_all(text, |caps: &regex::Captures| {
            if caps.name("empty_line").is_some() {
                "".to_string()
            } else if caps.name("excessive_newlines").is_some() {
                "\n\n".to_string()
            } else if let Some(trailing_match) = caps.name("trailing_space") {
                trailing_match.as_str().trim_end().to_string()
            } else if caps.name("leading_newlines").is_some() {
                "".to_string()
            } else if caps.name("last_whitespace").is_some() {
                "".to_string()
            } else if caps.name("empty_image").is_some() {
                "".to_string()
            } else if caps.name("leading_whitespace").is_some() {
                "".to_string()
            } else {
                caps[0].to_string()
            }
        })
        .to_string()
}

/// Intermediate result of HTML -> Markdown conversion.
///
/// Holds context in the form of parent tags and siblings chain
/// and resulting string of markup content with current position.
#[derive(Debug, Default)]
pub struct StructuredPrinter {
    /// Chain of parents leading to upmost <html> tag
    pub parent_chain: Vec<String>,
    /// Siblings of currently processed tag in order where they're appearing in html
    pub siblings: HashMap<usize, Vec<String>>,
    /// resulting markdown document
    pub data: String,
}

impl StructuredPrinter {
    /// Inserts newline
    pub fn insert_newline(&mut self) {
        self.append_str("\n");
    }

    /// Append string to the end of the printer
    pub fn append_str(&mut self, it: &str) {
        self.data.push_str(it);
    }

    /// Insert string at specified position of printer, adjust position to the end of inserted string
    pub fn insert_str(&mut self, pos: usize, it: &str) {
        self.data.insert_str(pos, it);
    }
}

/// Tag handler factory. This class is required in providing proper
/// custom tag parsing capabilities to users of this library.
///
/// The problem with directly providing tag handlers is that they're not stateless.
/// Once tag handler is parsing some tag, it holds data, such as start position, indent etc.
/// The only way to create fresh tag handler for each tag is to provide a factory like this one.
///
pub trait TagHandlerFactory {
    fn instantiate(&self) -> Box<dyn TagHandler>;
}

/// Trait interface describing abstract handler of arbitrary HTML tag.
pub trait TagHandler {
    /// Handle tag encountered when walking HTML tree.
    /// This is executed before the children processing
    fn handle(&mut self, tag: &Handle, printer: &mut StructuredPrinter);

    /// Executed after all children of this tag have been processed
    fn after_handle(&mut self, printer: &mut StructuredPrinter);

    fn skip_descendants(&self) -> bool {
        false
    }
}
