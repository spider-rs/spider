use super::StructuredPrinter;
use super::TagHandler;

use markup5ever_rcdom::{Handle, NodeData};

#[derive(Default)]
pub struct CodeHandler {
    code_type: String,
}

impl CodeHandler {
    /// Used in both starting and finishing handling
    fn do_handle(&mut self, printer: &mut StructuredPrinter, start: bool) {
        if self.code_type == "code"
            && match printer.parent_chain.last() {
                Some(s) => s == "pre",
                _ => false,
            }
        {
            // we are already in "code" mode
            return;
        }

        match self.code_type.as_ref() {
            "pre" => {
                // code block should have its own paragraph
                if start {
                    printer.insert_newline();
                }
                printer.append_str("\n```\n");
                if !start {
                    printer.insert_newline();
                }
            }
            "code" | "samp" => printer.append_str("`"),
            _ => (),
        }
    }
}

impl TagHandler for CodeHandler {
    fn handle(&mut self, tag: &Handle, printer: &mut StructuredPrinter) {
        self.code_type = match tag.data {
            NodeData::Element { ref name, .. } => name.local.to_string(),
            _ => String::new(),
        };

        self.do_handle(printer, true);
    }
    fn after_handle(&mut self, printer: &mut StructuredPrinter) {
        self.do_handle(printer, false);
    }
}
