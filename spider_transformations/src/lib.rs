pub mod html2text;
/// Html to xml.
pub mod html2xml;
/// rcdom
mod markup5ever_rcdom;
/// Base transformations.
pub mod transformation;
// shortcut
pub use transformation::content::{transform_content, transform_content_to_bytes};
