//! CSS. Functions (`rgb()`, `url()`), `@import`, and declaration names.

use tree_sitter::Node;

use super::{Adapter, ImportRef, Property, first_named_child, relative_path, text};
use crate::check::Project;

pub(super) struct Css;

impl Adapter for Css {
    fn raised_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// A function in a value: `rgb`, `calc`, `var`, `url`. Names are lowercased (CSS ignores case).
    fn call_name(&self, node: Node, src: &str) -> Option<String> {
        if node.kind() != "call_expression" {
            return None;
        }
        Some(text(node.child_by_field_name("function_name").or_else(|| first_named_child(node))?, src).to_ascii_lowercase())
    }

    fn call_arguments<'t>(&self, node: Node<'t>) -> Option<Vec<Node<'t>>> {
        if node.kind() != "call_expression" {
            return None;
        }
        let arguments = node.named_children(&mut node.walk()).find(|c| c.kind() == "arguments")?;
        let mut cursor = arguments.walk();
        Some(arguments.named_children(&mut cursor).filter(|n| n.kind() != "comment").collect())
    }

    fn string_value(&self, node: Node, src: &str) -> Option<String> {
        let raw = text(node, src);
        match node.kind() {
            "string_value" => raw.get(1..raw.len().checked_sub(1)?).filter(|s| !s.contains('\\')).map(str::to_string),
            "plain_value" => Some(raw.to_string()),
            _ => None,
        }
    }

    /// `@import "a.css"` and `@import url(a.css)`. A bare name is relative to the stylesheet, as in `./a.css`.
    fn imports(&self, node: Node, src: &str, file_path: &str, _project: &Project) -> Vec<ImportRef> {
        if node.kind() != "import_statement" {
            return Vec::new();
        }
        let Some(target) = first_named_child(node) else { return Vec::new() };
        let target = if target.kind() == "call_expression" {
            match self.call_arguments(target).and_then(|args| args.first().copied()) {
                Some(argument) => argument,
                None => return Vec::new(),
            }
        } else {
            target
        };
        let Some(specifier) = self.string_value(target, src) else { return Vec::new() };
        if specifier.contains("//") || specifier.starts_with("data:") {
            return vec![ImportRef { resolved: specifier.clone(), specifier }];
        }
        let local = if specifier.starts_with('.') { specifier.clone() } else { format!("./{specifier}") };
        let resolved = relative_path(&local, file_path).unwrap_or_else(|| specifier.clone());
        vec![ImportRef { specifier, resolved }]
    }

    /// A declaration is a property: `float: left`. Custom properties (`--x`) keep their case.
    fn object_property<'t>(&self, node: Node<'t>, src: &str) -> Option<Property<'t>> {
        if node.kind() != "declaration" {
            return None;
        }
        let name = text(first_named_child(node).filter(|n| n.kind() == "property_name")?, src);
        let key = if name.starts_with("--") { name.to_string() } else { name.to_ascii_lowercase() };
        Some(Property { key, value: None })
    }
}
