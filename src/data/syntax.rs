//! Small helpers for reading the tree-sitter nodes of the three formats.

use tree_sitter::Node as Syntax;

use super::model::{Node, Position, Scalar, Value};

pub(super) fn at(node: Syntax) -> Position {
    let p = node.start_position();
    (p.row + 1, p.column + 1)
}

pub(super) fn text<'a>(node: Syntax, src: &'a str) -> &'a str {
    &src[node.byte_range()]
}

pub(super) fn is_comment(node: Syntax) -> bool {
    node.kind().ends_with("comment")
}

pub(super) fn named_children(node: Syntax) -> Vec<Syntax> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).filter(|n| !is_comment(*n)).collect()
}

pub(super) fn scalar(at: Position, text: impl Into<String>) -> Node {
    Node { at, value: Value::Scalar(Scalar { text: text.into() }) }
}

pub(super) fn null(at: Position) -> Node {
    scalar(at, "null")
}

/// Resolves the escapes shared by JSON, YAML double quotes and TOML basic strings.
pub(super) fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some(u @ ('u' | 'U')) => {
                let length = if u == 'u' { 4 } else { 8 };
                let digits: String = chars.by_ref().take(length).collect();
                out.push(u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32).unwrap_or('\u{fffd}'));
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}
