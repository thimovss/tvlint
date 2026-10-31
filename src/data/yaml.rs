//! YAML syntax trees to the value model.

use tree_sitter::Node as Syntax;

use super::model::{Entry, Node, Value};
use super::syntax::{at, named_children, null, scalar, text, unescape};

pub(super) fn yaml_roots(stream: Syntax, src: &str) -> Vec<Node> {
    named_children(stream)
        .into_iter()
        .filter(|d| d.kind() == "document")
        .map(|document| {
            let body = named_children(document).into_iter().find(|c| matches!(c.kind(), "block_node" | "flow_node"));
            body.map_or_else(|| null(at(document)), |b| yaml_value(b, src))
        })
        .collect()
}

pub(super) fn yaml_value(node: Syntax, src: &str) -> Node {
    let position = at(node);
    match node.kind() {
        // Anchors, tags and comments wrap the real value.
        "block_node" | "flow_node" => named_children(node)
            .into_iter()
            .find(|c| !matches!(c.kind(), "anchor" | "tag"))
            .map_or_else(|| null(position), |inner| yaml_value(inner, src)),
        "block_mapping" | "flow_mapping" => {
            let mut entries = Vec::new();
            for pair in named_children(node) {
                match pair.kind() {
                    "block_mapping_pair" | "flow_pair" => {
                        let Some(key) = pair.child_by_field_name("key") else { continue };
                        let value = pair.child_by_field_name("value").map_or_else(|| null(at(pair)), |v| yaml_value(v, src));
                        entries.push(Entry { key: yaml_key(key, src), at: at(key), value });
                    }
                    // `{a, b}`: keys without values.
                    "flow_node" => entries.push(Entry { key: yaml_key(pair, src), at: at(pair), value: null(at(pair)) }),
                    _ => {}
                }
            }
            Node { at: position, value: Value::Map(entries) }
        }
        "block_sequence" => Node {
            at: position,
            value: Value::Seq(
                named_children(node)
                    .into_iter()
                    .filter(|c| c.kind() == "block_sequence_item")
                    .map(|item| {
                        named_children(item).into_iter().next().map_or_else(|| null(at(item)), |inner| yaml_value(inner, src))
                    })
                    .collect(),
            ),
        },
        "flow_sequence" => Node {
            at: position,
            value: Value::Seq(
                named_children(node)
                    .into_iter()
                    .map(|item| match item.kind() {
                        "flow_pair" => {
                            let key = item.child_by_field_name("key");
                            let value = item.child_by_field_name("value").map_or_else(|| null(at(item)), |v| yaml_value(v, src));
                            let entries = key.map(|k| Entry { key: yaml_key(k, src), at: at(k), value }).into_iter().collect();
                            Node { at: at(item), value: Value::Map(entries) }
                        }
                        _ => yaml_value(item, src),
                    })
                    .collect(),
            ),
        },
        "plain_scalar" => {
            let raw = text(node, src).trim();
            scalar(position, raw)
        }
        "double_quote_scalar" => scalar(position, unescape(strip_quotes(text(node, src)))),
        "single_quote_scalar" => scalar(position, strip_quotes(text(node, src)).replace("''", "'")),
        "block_scalar" => scalar(position, block_scalar_text(text(node, src))),
        "alias" => scalar(position, text(node, src).trim()),
        _ => null(position),
    }
}

pub(super) fn strip_quotes(raw: &str) -> &str {
    raw.get(1..raw.len().saturating_sub(1)).unwrap_or("")
}

pub(super) fn yaml_key(node: Syntax, src: &str) -> String {
    match yaml_value(node, src).value {
        Value::Scalar(s) => s.text,
        _ => text(node, src).trim().to_string(),
    }
}

/// The lines of a `|` or `>` block without the header, de-indented.
pub(super) fn block_scalar_text(raw: &str) -> String {
    let body = raw.split_once('\n').map_or("", |(_, rest)| rest);
    let indent = body.lines().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    body.lines().map(|l| l.get(indent..).unwrap_or("").trim_end()).collect::<Vec<_>>().join("\n").trim_end().to_string()
}
