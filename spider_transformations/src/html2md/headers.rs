use super::StructuredPrinter;
use super::TagHandler;

use markup5ever_rcdom::{Handle, NodeData};

#[derive(Default)]
pub struct HeaderHandler {
    header_type: String,
}

impl TagHandler for HeaderHandler {
    fn handle(&mut self, tag: &Handle, printer: &mut StructuredPrinter) {
        self.header_type = match tag.data {
            NodeData::Element { ref name, .. } => name.local.to_string(),
            _ => String::new(),
        };

        match self.header_type.as_ref() {
            "h1" => printer.append_str("# "),
            "h2" => printer.append_str("## "),
            "h3" => printer.append_str("### "),
            "h4" => printer.append_str("#### "),
            "h5" => printer.append_str("##### "),
            "h6" => printer.append_str("###### "),
            _ => (),
        }
    }

    fn after_handle(&mut self, printer: &mut StructuredPrinter) {
        printer.insert_newline();
    }
}
