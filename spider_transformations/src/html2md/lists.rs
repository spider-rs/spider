use super::StructuredPrinter;
use super::TagHandler;

use markup5ever_rcdom::Handle;

/// gets all list elements registered by a `StructuredPrinter` in reverse order
fn list_hierarchy(printer: &mut StructuredPrinter) -> Vec<&String> {
    printer
        .parent_chain
        .iter()
        .rev()
        .filter(|&tag| tag == "ul" || tag == "ol" || tag == "menu")
        .collect()
}

#[derive(Default)]
pub struct ListHandler;

impl TagHandler for ListHandler {
    /// we're entering "ul" or "ol" tag, no "li" handling here
    fn handle(&mut self, _tag: &Handle, printer: &mut StructuredPrinter) {
        printer.insert_newline();

        // insert an extra newline for non-nested lists
        if list_hierarchy(printer).is_empty() {
            printer.insert_newline();
        }
    }

    /// indent now-ready list
    fn after_handle(&mut self, printer: &mut StructuredPrinter) {
        printer.insert_newline();
    }
}

#[derive(Default)]
pub struct ListItemHandler {
    start_pos: usize,
    list_type: String,
}

impl TagHandler for ListItemHandler {
    fn handle(&mut self, _tag: &Handle, printer: &mut StructuredPrinter) {
        {
            let parent_lists = list_hierarchy(printer);
            let nearest_parent_list = parent_lists.first();
            if nearest_parent_list.is_none() {
                // no parent list
                // should not happen - html5ever cleans html input when parsing
                return;
            }

            match nearest_parent_list {
                Some(s) => {
                    self.list_type = s.to_string();
                }
                _ => (),
            }
        }

        if printer.data.chars().last() != Some('\n') {
            // insert newline when declaring a list item only in case there isn't any newline at the end of text
            printer.insert_newline();
        }

        let current_depth = printer.parent_chain.len();
        let order = printer.siblings[&current_depth].len() + 1;
        match self.list_type.as_ref() {
            "ul" | "menu" => printer.append_str("* "), // unordered list: *, *, *
            "ol" => printer.append_str(&(order.to_string() + ". ")), // ordered list: 1, 2, 3
            _ => (),                                   // never happens
        }

        self.start_pos = printer.data.len();
    }

    fn after_handle(&mut self, printer: &mut StructuredPrinter) {
        let padding = match self.list_type.as_ref() {
            "ul" => 2,
            "ol" => 3,
            _ => 4,
        };

        // need to cleanup leading newlines, <p> inside <li> should produce valid
        // list element, not an empty line
        let index = self.start_pos;
        while index < printer.data.len() {
            if printer.data.bytes().nth(index) == Some(b'\n')
                || printer.data.bytes().nth(index) == Some(b' ')
            {
                printer.data.remove(index);
            } else {
                break;
            }
        }

        // non-nested indentation (padding). Markdown requires that all paragraphs in the
        // list item except first should be indented with at least 1 space
        let mut index = printer.data.len();
        while index > self.start_pos {
            if printer.data.bytes().nth(index) == Some(b'\n') {
                printer.insert_str(index + 1, &" ".repeat(padding));
            }
            index -= 1;
        }
    }
}
