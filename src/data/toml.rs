//! TOML syntax trees to the value model.

use tree_sitter::Node as Syntax;

use super::model::{Entry, Node, Position, Value};
use super::syntax::{at, named_children, scalar, text, unescape};
use super::yaml::strip_quotes;

pub(super) type KeyPath = Vec<(String, Position)>;

pub(super) fn toml_root(document: Syntax, src: &str) -> Node {
    let mut top: Vec<Entry> = Vec::new();
    for child in named_children(document) {
        match child.kind() {
            "pair" => toml_insert(&mut top, child, src),
            "table" => {
                let path = toml_header(child, src);
                let entries = toml_navigate(&mut top, &path);
                for pair in named_children(child).into_iter().filter(|c| c.kind() == "pair") {
                    toml_insert(entries, pair, src);
                }
            }
            "table_array_element" => {
                let mut path = toml_header(child, src);
                let Some((name, position)) = path.pop() else { continue };
                let parent = toml_navigate(&mut top, &path);
                let index = match parent.iter().position(|e| e.key == name) {
                    Some(i) => i,
                    None => {
                        parent.push(Entry { key: name, at: position, value: Node { at: position, value: Value::Seq(Vec::new()) } });
                        parent.len() - 1
                    }
                };
                if !matches!(parent[index].value.value, Value::Seq(_)) {
                    parent[index].value.value = Value::Seq(Vec::new());
                }
                let Value::Seq(items) = &mut parent[index].value.value else { continue };
                items.push(Node { at: at(child), value: Value::Map(Vec::new()) });
                if let Some(Value::Map(entries)) = items.last_mut().map(|n| &mut n.value) {
                    for pair in named_children(child).into_iter().filter(|c| c.kind() == "pair") {
                        toml_insert(entries, pair, src);
                    }
                }
            }
            _ => {}
        }
    }
    Node { at: (1, 1), value: Value::Map(top) }
}

/// The keys of a `[a.b]` or `[[a.b]]` header.
pub(super) fn toml_header(table: Syntax, src: &str) -> KeyPath {
    named_children(table).into_iter().next().map(|key| toml_keys(key, src)).unwrap_or_default()
}

pub(super) fn toml_keys(key: Syntax, src: &str) -> KeyPath {
    match key.kind() {
        "dotted_key" => named_children(key).into_iter().flat_map(|k| toml_keys(k, src)).collect(),
        "quoted_key" => vec![(unescape(strip_quotes(text(key, src))), at(key))],
        _ => vec![(text(key, src).to_string(), at(key))],
    }
}

/// The entries of the table at `path`, created if missing. The last item of an array of tables counts.
pub(super) fn toml_navigate<'a>(mut entries: &'a mut Vec<Entry>, path: &KeyPath) -> &'a mut Vec<Entry> {
    for (name, position) in path {
        let index = match entries.iter().position(|e| &e.key == name) {
            Some(i) => i,
            None => {
                entries.push(Entry { key: name.clone(), at: *position, value: Node { at: *position, value: Value::Map(Vec::new()) } });
                entries.len() - 1
            }
        };
        let node = &mut entries[index].value;
        if matches!(node.value, Value::Scalar(_)) {
            node.value = Value::Map(Vec::new());
        }
        if let Value::Seq(items) = &mut node.value
            && !matches!(items.last().map(|n| &n.value), Some(Value::Map(_)))
        {
            items.push(Node { at: *position, value: Value::Map(Vec::new()) });
        }
        entries = match &mut node.value {
            Value::Map(m) => m,
            Value::Seq(items) => match items.last_mut().map(|n| &mut n.value) {
                Some(Value::Map(m)) => m,
                _ => unreachable!("an array of tables ends in a table"),
            },
            Value::Scalar(_) => unreachable!("scalars were replaced above"),
        };
    }
    entries
}

/// `key = value`, where the key may be dotted.
pub(super) fn toml_insert(entries: &mut Vec<Entry>, pair: Syntax, src: &str) {
    let children = named_children(pair);
    let (Some(key), Some(value)) = (children.first(), children.last()) else { return };
    let mut path = toml_keys(*key, src);
    let Some((name, position)) = path.pop() else { return };
    let value = toml_value(*value, src);
    toml_navigate(entries, &path).push(Entry { key: name, at: position, value });
}

pub(super) fn toml_value(node: Syntax, src: &str) -> Node {
    let position = at(node);
    match node.kind() {
        "string" => scalar(position, toml_string(text(node, src))),
        "integer" | "float" => scalar(position, text(node, src)),
        "boolean" => scalar(position, text(node, src)),
        "array" => Node { at: position, value: Value::Seq(named_children(node).into_iter().map(|c| toml_value(c, src)).collect()) },
        "inline_table" => {
            let mut entries = Vec::new();
            for pair in named_children(node).into_iter().filter(|c| c.kind() == "pair") {
                toml_insert(&mut entries, pair, src);
            }
            Node { at: position, value: Value::Map(entries) }
        }
        _ => scalar(position, text(node, src)),
    }
}

pub(super) fn toml_string(raw: &str) -> String {
    for (quote, literal) in [("\"\"\"", false), ("'''", true), ("\"", false), ("'", true)] {
        if let Some(inner) = raw.strip_prefix(quote).and_then(|r| r.strip_suffix(quote)) {
            let inner = inner.strip_prefix('\n').unwrap_or(inner);
            return if literal { inner.to_string() } else { unescape(inner) };
        }
    }
    raw.to_string()
}
