use super::markup5ever_rcdom::{Handle, NodeData, RcDom};
use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, QualName};
use markup5ever::namespace_url;
use markup5ever::ns;
use spider::page::get_html_encoded;
use std::default::Default;
use std::error::Error;
use std::io::{self, Write};

/// Convert HTML to well-formed XML.
pub fn convert_html_to_xml(
    html: &str,
    url: &str,
    encoding: &Option<String>,
) -> Result<String, Box<dyn Error>> {
    let parser = parse_document(RcDom::default(), Default::default());
    let dom = parser.one(html);
    let mut xml_output = Vec::new();
    let encoding = if let Some(ref encoding) = encoding {
        encoding
    } else {
        "UTF-8"
    };
    let root = format!(r#"<?xml version="1.0" encoding="{encoding}"?><root xmlns:custom="{url}">"#);

    write!(xml_output, "{root}")?;
    serialize_xml(&dom.document, &mut xml_output)?;
    write!(xml_output, "</root>")?;

    Ok(get_html_encoded(&Some(xml_output.into()), &encoding))
}

/// Serialize a DOM node into XML.
fn serialize_xml<W: Write>(handle: &Handle, writer: &mut W) -> io::Result<()> {
    match handle.data {
        NodeData::Document => {
            for child in handle.children.borrow().iter() {
                serialize_xml(child, writer)?;
            }
        }
        NodeData::Element {
            ref name,
            ref attrs,
            ..
        } => {
            write!(writer, "<{}", qual_name_to_string(name))?;

            for attr in attrs.borrow().iter() {
                let attr_name = qual_name_to_string(&attr.name);
                let processed_name = if attr_name.contains(":") {
                    format!("custom:{}", attr_name.replace(":", ""))
                } else {
                    attr_name
                };

                write!(
                    writer,
                    " {}=\"{}\"",
                    processed_name,
                    escape_xml(&attr.value)
                )?;
            }

            let children = handle.children.borrow();
            if children.is_empty() {
                write!(writer, " />")?;
            } else {
                write!(writer, ">")?;
                for child in children.iter() {
                    serialize_xml(child, writer)?;
                }
                write!(writer, "</{}>", qual_name_to_string(name))?;
            }
        }
        NodeData::Text { ref contents } => {
            write!(writer, "{}", escape_xml(&contents.borrow()))?;
        }
        NodeData::Comment { ref contents } => {
            write!(writer, "<!--{}-->", escape_xml(&contents.to_string()))?;
        }
        NodeData::Doctype { ref name, .. } => {
            write!(writer, "<!DOCTYPE {}>", name)?;
        }
        _ => (),
    }
    Ok(())
}

/// Helper function to convert qualified names into a string representation.
fn qual_name_to_string(name: &QualName) -> String {
    if name.ns == ns!(html) {
        name.local.to_string()
    } else {
        format!("{}:{}", name.ns.to_string(), name.local)
    }
}

/// Escape special characters for XML documents.
fn escape_xml(text: &str) -> String {
    text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("\"", "&quot;")
        .replace("'", "&apos;")
}
