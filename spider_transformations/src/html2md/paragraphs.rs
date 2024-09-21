use super::StructuredPrinter;
use super::TagHandler;

use markup5ever_rcdom::{Handle, NodeData};

#[derive(Default)]
pub struct ParagraphHandler {
    paragraph_type: String,
}

impl TagHandler for ParagraphHandler {
    fn handle(&mut self, tag: &Handle, printer: &mut StructuredPrinter) {
        self.paragraph_type = match tag.data {
            NodeData::Element { ref name, .. } => name.local.to_string(),
            _ => String::new(),
        };

        // insert newlines at the start of paragraph
        match self.paragraph_type.as_ref() {
            "p" => {
                printer.insert_newline();
            }
            _ => (),
        }
    }

    fn after_handle(&mut self, printer: &mut StructuredPrinter) {
        // insert newlines at the end of paragraph
        match self.paragraph_type.as_ref() {
            "p" => {
                printer.insert_newline();
            }
            "hr" => {
                printer.insert_newline();
                printer.append_str("---");
                printer.insert_newline();
            }
            "br" => printer.append_str("\n"), // we prob want nbsp here.
            _ => (),
        }
    }
}
