use markup5ever_rcdom::{Handle, NodeData};

pub fn get_tag_attr(tag: &Handle, attr_name: &str) -> Option<String> {
    match tag.data {
        NodeData::Element { ref attrs, .. } => {
            let attrs = attrs.borrow();
            let requested_attr = attrs
                .iter()
                .find(|attr| attr.name.local.as_bytes() == attr_name.as_bytes());

            requested_attr.map(|attr| attr.value.to_string())
        }
        _ => None,
    }
}
