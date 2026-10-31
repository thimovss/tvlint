//! The value model: maps, sequences and scalars, each with its position, and the walks over it.

use std::collections::BTreeMap;

use crate::config::{Language, Use};
use crate::source::SourceFile;

use super::json::json_root;
use super::path::PathPattern;
use super::toml::toml_root;
use super::yaml::yaml_roots;

/// 1-based line and column.
pub type Position = (usize, usize);

pub struct Node {
    pub at: Position,
    pub value: Value,
}

pub enum Value {
    Map(Vec<Entry>),
    Seq(Vec<Node>),
    Scalar(Scalar),
}

pub struct Entry {
    pub key: String,
    pub at: Position,
    pub value: Node,
}

pub struct Scalar {
    /// The decoded text: quotes removed, escapes resolved. `null` for a null. Numbers, booleans,
    /// dates and strings are all just text, so `true` and `"true"` are the same to a rule.
    pub text: String,
}

impl Node {
    pub fn map(&self) -> Option<&[Entry]> {
        match &self.value {
            Value::Map(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn scalar(&self) -> Option<&Scalar> {
        match &self.value {
            Value::Scalar(scalar) => Some(scalar),
            _ => None,
        }
    }
}

/// A parsed file. YAML can hold several documents; JSON and TOML always hold one (or none, if empty).
pub struct Doc {
    pub roots: Vec<Node>,
}

impl Doc {
    pub fn build(file: &SourceFile) -> Doc {
        let root = file.tree.root_node();
        let src = file.text.as_str();
        let roots = match file.language {
            Language::Json => json_root(root, src).into_iter().collect(),
            Language::Yaml => yaml_roots(root, src),
            Language::Toml => vec![toml_root(root, src)],
            _ => Vec::new(),
        };
        Doc { roots }
    }

    /// Calls `f` for every node, root first: with the keys leading to it (array items count as their
    /// index), the position to report it at (the key's, for a map value) and the node itself.
    pub fn walk(&self, f: &mut dyn FnMut(&[String], Position, &Node)) {
        for root in &self.roots {
            let mut path = Vec::new();
            f(&path, root.at, root);
            walk_node(root, &mut path, f);
        }
    }

    /// The allowed names a registry file offers: what `pattern` selects, read as `use_`.
    pub fn registry(&self, pattern: &PathPattern, use_: Use) -> BTreeMap<String, Position> {
        let mut found = BTreeMap::new();
        self.walk(&mut |path, at, node| {
            if path.is_empty() || !pattern.matches(path) {
                return;
            }
            let name = match use_ {
                Use::Key => path.last().cloned(),
                Use::Value => node.scalar().map(|s| s.text.clone()),
                Use::Path => node.scalar().map(|_| path.join(".")),
            };
            if let Some(name) = name {
                found.entry(name).or_insert(at);
            }
        });
        found
    }

    /// Every value that is not a map, by its keys: the "keys" two translation files must share.
    /// Kept as segments, so `{"a.b": 1}` and `{"a": {"b": 1}}` stay different.
    pub fn leaves(&self) -> BTreeMap<Vec<String>, (Position, Option<String>)> {
        let mut found = BTreeMap::new();
        if let Some(first) = self.roots.first() {
            collect_leaves(first, &mut Vec::new(), &mut found);
        }
        found
    }

    /// The position of the deepest ancestor of `path` that exists, to report a missing key near.
    pub fn deepest_existing(&self, path: &[String]) -> Position {
        let Some(first) = self.roots.first() else { return (1, 1) };
        let mut at = first.at;
        let mut node = first;
        for segment in path {
            let Some(entry) = node.map().and_then(|entries| entries.iter().find(|e| e.key == *segment)) else { break };
            at = entry.at;
            node = &entry.value;
        }
        at
    }
}

/// A path for messages, in the syntax of `PathPattern`: a key that contains a dot is quoted, so
/// `["a.b", "c"]` reads `"a.b".c` and differs from `["a", "b", "c"]`.
pub fn dotted(path: &[String]) -> String {
    let shown: Vec<String> = path.iter().map(|s| if s.contains('.') { format!("\"{s}\"") } else { s.clone() }).collect();
    shown.join(".")
}

pub(super) fn walk_node(node: &Node, path: &mut Vec<String>, f: &mut dyn FnMut(&[String], Position, &Node)) {
    match &node.value {
        Value::Map(entries) => {
            for entry in entries {
                path.push(entry.key.clone());
                f(path, entry.at, &entry.value);
                walk_node(&entry.value, path, f);
                path.pop();
            }
        }
        Value::Seq(items) => {
            for (i, item) in items.iter().enumerate() {
                path.push(i.to_string());
                f(path, item.at, item);
                walk_node(item, path, f);
                path.pop();
            }
        }
        Value::Scalar(_) => {}
    }
}

pub(super) fn collect_leaves(node: &Node, path: &mut Vec<String>, out: &mut BTreeMap<Vec<String>, (Position, Option<String>)>) {
    let Value::Map(entries) = &node.value else { return };
    for entry in entries {
        path.push(entry.key.clone());
        match &entry.value.value {
            Value::Map(_) => collect_leaves(&entry.value, path, out),
            Value::Seq(_) => {
                out.insert(path.clone(), (entry.at, None));
            }
            Value::Scalar(s) => {
                out.insert(path.clone(), (entry.at, Some(s.text.clone())));
            }
        }
        path.pop();
    }
}
