use super::StructuredPrinter;
use super::TagHandler;
use markup5ever_rcdom::Handle;

#[derive(Default)]
pub struct ContainerHandler;

impl TagHandler for ContainerHandler {
    fn handle(&mut self, _tag: &Handle, printer: &mut StructuredPrinter) {
        printer.insert_newline();
    }

    fn after_handle(&mut self, printer: &mut StructuredPrinter) {
        printer.insert_newline();
    }
}
