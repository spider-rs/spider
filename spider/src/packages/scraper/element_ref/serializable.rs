use std::io::Error;

use fast_html5ever::serialize::{Serialize, Serializer, TraversalScope};

use super::ElementRef;

impl<'a> Serialize for ElementRef<'a> {
    fn serialize<S: Serializer>(
        &self,
        serializer: &mut S,
        traversal_scope: TraversalScope,
    ) -> Result<(), Error> {
        super::super::node::serializable::serialize(**self, serializer, traversal_scope)
    }
}
