use super::StructuredPrinter;
use super::TagHandler;

use markup5ever_rcdom::{Handle, NodeData};

#[derive(Default)]
pub struct AnchorHandler {
    start_pos: usize,
    url: String,
}

impl TagHandler for AnchorHandler {
    fn handle(&mut self, tag: &Handle, printer: &mut StructuredPrinter) {
        self.start_pos = printer.data.len();

        // try to extract a hyperlink
        self.url = match tag.data {
            NodeData::Element { ref attrs, .. } => {
                let attrs = attrs.borrow();
                let href = attrs
                    .iter()
                    .find(|attr| attr.name.local.as_bytes() == b"href");

                match href {
                    Some(link) => link.value.trim().into(),
                    None => String::new(),
                }
            }
            _ => String::new(),
        };
    }

    fn after_handle(&mut self, printer: &mut StructuredPrinter) {
        // add braces around already present text, put an url afterwards
        printer.insert_str(self.start_pos, "[");
        printer.append_str(&format!("]({})", self.url))
    }
}
