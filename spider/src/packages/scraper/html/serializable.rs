use std::io::Error;

use html5ever::serialize::{Serialize, Serializer, TraversalScope};

use super::Html;

impl Serialize for Html {
    fn serialize<S: Serializer>(
        &self,
        serializer: &mut S,
        traversal_scope: TraversalScope,
    ) -> Result<(), Error> {
        super::super::node::serializable::serialize(self.tree.root(), serializer, traversal_scope)
    }
}

#[cfg(test)]
mod tests {
    use super::Html;

    #[test]
    fn test_serialize() {
        let src = r#"<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"></head><body><p>Hello world!</p></body></html>"#;
        let html = Html::parse_document(src);
        assert_eq!(html.html(), src);
    }
}
