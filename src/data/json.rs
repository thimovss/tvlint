//! JSON (and JSONC) syntax trees to the value model.

use tree_sitter::Node as Syntax;

use super::model::{Entry, Node, Value};
use super::syntax::{at, named_children, null, scalar, text, unescape};

pub(super) fn json_root(document: Syntax, src: &str) -> Option<Node> {
    named_children(document).into_iter().find_map(|child| json_value(child, src))
}

pub(super) fn json_value(node: Syntax, src: &str) -> Option<Node> {
    let position = at(node);
    Some(match node.kind() {
        "object" => {
            let mut entries = Vec::new();
            for pair in named_children(node).into_iter().filter(|c| c.kind() == "pair") {
                let key = pair.child_by_field_name("key")?;
                let value = pair.child_by_field_name("value").and_then(|v| json_value(v, src)).unwrap_or_else(|| null(at(pair)));
                entries.push(Entry { key: json_string(key, src), at: at(key), value });
            }
            Node { at: position, value: Value::Map(entries) }
        }
        "array" => Node {
            at: position,
            value: Value::Seq(named_children(node).into_iter().filter_map(|c| json_value(c, src)).collect()),
        },
        "string" => scalar(position, json_string(node, src)),
        "number" => scalar(position, text(node, src)),
        "true" | "false" => scalar(position, text(node, src)),
        "null" => null(position),
        _ => return None,
    })
}

pub(super) fn json_string(node: Syntax, src: &str) -> String {
    let raw = text(node, src);
    unescape(raw.get(1..raw.len().saturating_sub(1)).unwrap_or(""))
}
