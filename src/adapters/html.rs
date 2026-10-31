//! HTML. Markup has no calls, raises or imports: the checks are elements and attributes.

use tree_sitter::Node;

use super::{Adapter, Property, text};

pub(super) struct Html;

impl Adapter for Html {
    fn raised_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    fn call_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// An opening or self-closing tag, named in lowercase because HTML tag names are case-insensitive.
    fn element_name(&self, node: Node, src: &str) -> Option<String> {
        if !matches!(node.kind(), "start_tag" | "self_closing_tag") {
            return None;
        }
        let mut cursor = node.walk();
        let name = node.named_children(&mut cursor).find(|c| c.kind() == "tag_name")?;
        Some(text(name, src).to_ascii_lowercase())
    }

    /// An attribute is a property: `class="x"`, or a bare `disabled` with no value.
    fn object_property<'t>(&self, node: Node<'t>, src: &str) -> Option<Property<'t>> {
        if node.kind() != "attribute" {
            return None;
        }
        let mut cursor = node.walk();
        let mut children = node.named_children(&mut cursor);
        let name = children.find(|c| c.kind() == "attribute_name")?;
        let value = children.find(|c| matches!(c.kind(), "quoted_attribute_value" | "attribute_value"));
        Some(Property { key: text(name, src).to_ascii_lowercase(), value })
    }

    fn is_string_literal(&self, node: Node, _src: &str) -> bool {
        matches!(node.kind(), "quoted_attribute_value" | "attribute_value")
    }
}
