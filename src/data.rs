//! JSON, YAML and TOML as one value model: maps, sequences and scalars, each with its position.
//!
//! This is the data counterpart of the language adapters. It is the only place that knows the
//! tree-sitter node kinds of the three formats, so rules can ask questions by path
//! (`jobs.*.runs-on`) without caring which format the file is written in.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use globset::{Glob, GlobMatcher};
use tree_sitter::Node as Syntax;

use crate::config::{Language, Use};
use crate::source::SourceFile;

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

fn walk_node(node: &Node, path: &mut Vec<String>, f: &mut dyn FnMut(&[String], Position, &Node)) {
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

fn collect_leaves(node: &Node, path: &mut Vec<String>, out: &mut BTreeMap<Vec<String>, (Position, Option<String>)>) {
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

// ---------------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------------

enum Segment {
    Literal(String),
    Glob(GlobMatcher),
    /// `*`: any one key or array index.
    Any,
    /// `**`: any number of keys, including none.
    Deep,
}

/// A dotted path with wildcards, anchored at the root of the file: `scripts.postinstall`,
/// `jobs.*.runs-on`, `**.version`. Array items are addressed by index, so `*` also steps into lists.
/// A key that contains a dot is written in double quotes: `"a.b".c`.
pub struct PathPattern {
    text: String,
    segments: Vec<Segment>,
}

impl PathPattern {
    pub fn parse(text: &str) -> Result<PathPattern> {
        let mut segments = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut was_quoted = false;
        let mut finish = |current: &mut String, was_quoted: &mut bool| -> Result<()> {
            if current.is_empty() {
                bail!("`{text}` has an empty segment");
            }
            let segment = if *was_quoted {
                Segment::Literal(std::mem::take(current))
            } else {
                match current.as_str() {
                    "*" => Segment::Any,
                    "**" => Segment::Deep,
                    s if s.contains(['*', '?', '[']) => Segment::Glob(Glob::new(s)?.compile_matcher()),
                    _ => Segment::Literal(current.clone()),
                }
            };
            current.clear();
            *was_quoted = false;
            segments.push(segment);
            Ok(())
        };
        for c in text.chars() {
            match c {
                '"' => {
                    quoted = !quoted;
                    was_quoted = true;
                }
                '.' if !quoted => finish(&mut current, &mut was_quoted)?,
                c => current.push(c),
            }
        }
        if quoted {
            bail!("`{text}` has an unclosed quote");
        }
        finish(&mut current, &mut was_quoted)?;
        Ok(PathPattern { text: text.to_string(), segments })
    }

    pub fn matches(&self, path: &[String]) -> bool {
        matches_from(&self.segments, path)
    }

    /// Whether the pattern names exactly one place, with no wildcard.
    pub fn is_literal(&self) -> bool {
        self.segments.iter().all(|s| matches!(s, Segment::Literal(_)))
    }

    /// The pattern for the container, and the key that must be in it: `jobs.*.runs-on` is
    /// (`jobs.*`, `runs-on`). `None` if the last segment is a wildcard, which names no single key.
    pub fn split_last(&self) -> Option<(PathPattern, String)> {
        let (last, parent) = self.segments.split_last()?;
        let Segment::Literal(key) = last else { return None };
        let text = self.text.rsplit_once('.').map_or(String::new(), |(parent, _)| parent.to_string());
        let parent = PathPattern {
            text,
            segments: parent
                .iter()
                .map(|s| match s {
                    Segment::Literal(l) => Segment::Literal(l.clone()),
                    Segment::Glob(g) => Segment::Glob(g.glob().compile_matcher()),
                    Segment::Any => Segment::Any,
                    Segment::Deep => Segment::Deep,
                })
                .collect(),
        };
        Some((parent, key.clone()))
    }
}

fn matches_from(segments: &[Segment], path: &[String]) -> bool {
    let Some((first, rest)) = segments.split_first() else { return path.is_empty() };
    match first {
        Segment::Deep => (0..=path.len()).any(|skip| matches_from(rest, &path[skip..])),
        _ => {
            let Some((key, remaining)) = path.split_first() else { return false };
            let hit = match first {
                Segment::Literal(l) => l == key,
                Segment::Glob(g) => g.is_match(key),
                _ => true,
            };
            hit && matches_from(rest, remaining)
        }
    }
}

/// The names between `{{ }}` or `{ }` in a message: `Hello {{name}}, {count, plural, ...}` has
/// `name` and `count`. Two translations of one message must use the same ones.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '{' {
            i += 1;
            continue;
        }
        let double = chars.get(i + 1) == Some(&'{');
        let start = i + if double { 2 } else { 1 };
        // The group ends at the matching brace; nested groups (plural branches) belong to it.
        let mut depth = 1;
        let mut end = start;
        while end < chars.len() {
            match chars[end] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        let inner: String = chars[start..end.min(chars.len())].iter().collect();
        let name = inner.split(',').next().unwrap_or("").trim();
        if !name.is_empty() && !name.contains(char::is_whitespace) {
            found.insert(name.to_string());
        }
        i = end + 1;
    }
    found
}

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

fn at(node: Syntax) -> Position {
    let p = node.start_position();
    (p.row + 1, p.column + 1)
}

fn text<'a>(node: Syntax, src: &'a str) -> &'a str {
    &src[node.byte_range()]
}

fn is_comment(node: Syntax) -> bool {
    node.kind().ends_with("comment")
}

fn named_children(node: Syntax) -> Vec<Syntax> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).filter(|n| !is_comment(*n)).collect()
}

fn scalar(at: Position, text: impl Into<String>) -> Node {
    Node { at, value: Value::Scalar(Scalar { text: text.into() }) }
}

fn null(at: Position) -> Node {
    scalar(at, "null")
}

/// Resolves the escapes shared by JSON, YAML double quotes and TOML basic strings.
fn unescape(raw: &str) -> String {
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

// ---------------------------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------------------------

fn json_root(document: Syntax, src: &str) -> Option<Node> {
    named_children(document).into_iter().find_map(|child| json_value(child, src))
}

fn json_value(node: Syntax, src: &str) -> Option<Node> {
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

fn json_string(node: Syntax, src: &str) -> String {
    let raw = text(node, src);
    unescape(raw.get(1..raw.len().saturating_sub(1)).unwrap_or(""))
}

// ---------------------------------------------------------------------------------------------
// YAML
// ---------------------------------------------------------------------------------------------

fn yaml_roots(stream: Syntax, src: &str) -> Vec<Node> {
    named_children(stream)
        .into_iter()
        .filter(|d| d.kind() == "document")
        .map(|document| {
            let body = named_children(document).into_iter().find(|c| matches!(c.kind(), "block_node" | "flow_node"));
            body.map_or_else(|| null(at(document)), |b| yaml_value(b, src))
        })
        .collect()
}

fn yaml_value(node: Syntax, src: &str) -> Node {
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

fn strip_quotes(raw: &str) -> &str {
    raw.get(1..raw.len().saturating_sub(1)).unwrap_or("")
}

fn yaml_key(node: Syntax, src: &str) -> String {
    match yaml_value(node, src).value {
        Value::Scalar(s) => s.text,
        _ => text(node, src).trim().to_string(),
    }
}

/// The lines of a `|` or `>` block without the header, de-indented.
fn block_scalar_text(raw: &str) -> String {
    let body = raw.split_once('\n').map_or("", |(_, rest)| rest);
    let indent = body.lines().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    body.lines().map(|l| l.get(indent..).unwrap_or("").trim_end()).collect::<Vec<_>>().join("\n").trim_end().to_string()
}

// ---------------------------------------------------------------------------------------------
// TOML
// ---------------------------------------------------------------------------------------------

type KeyPath = Vec<(String, Position)>;

fn toml_root(document: Syntax, src: &str) -> Node {
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
fn toml_header(table: Syntax, src: &str) -> KeyPath {
    named_children(table).into_iter().next().map(|key| toml_keys(key, src)).unwrap_or_default()
}

fn toml_keys(key: Syntax, src: &str) -> KeyPath {
    match key.kind() {
        "dotted_key" => named_children(key).into_iter().flat_map(|k| toml_keys(k, src)).collect(),
        "quoted_key" => vec![(unescape(strip_quotes(text(key, src))), at(key))],
        _ => vec![(text(key, src).to_string(), at(key))],
    }
}

/// The entries of the table at `path`, created if missing. The last item of an array of tables counts.
fn toml_navigate<'a>(mut entries: &'a mut Vec<Entry>, path: &KeyPath) -> &'a mut Vec<Entry> {
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
fn toml_insert(entries: &mut Vec<Entry>, pair: Syntax, src: &str) {
    let children = named_children(pair);
    let (Some(key), Some(value)) = (children.first(), children.last()) else { return };
    let mut path = toml_keys(*key, src);
    let Some((name, position)) = path.pop() else { return };
    let value = toml_value(*value, src);
    toml_navigate(entries, &path).push(Entry { key: name, at: position, value });
}

fn toml_value(node: Syntax, src: &str) -> Node {
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

fn toml_string(raw: &str) -> String {
    for (quote, literal) in [("\"\"\"", false), ("'''", true), ("\"", false), ("'", true)] {
        if let Some(inner) = raw.strip_prefix(quote).and_then(|r| r.strip_suffix(quote)) {
            let inner = inner.strip_prefix('\n').unwrap_or(inner);
            return if literal { inner.to_string() } else { unescape(inner) };
        }
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Parsers;

    fn doc(path: &str, text: &str) -> Doc {
        let (language, tree) = Parsers::new().unwrap().parse(path, text).unwrap();
        Doc::build(&SourceFile { path: path.into(), language, text: text.into(), tree })
    }

    /// Every node as `path = value` (or `path` for containers), in document order.
    fn dump(doc: &Doc) -> Vec<String> {
        let mut out = Vec::new();
        doc.walk(&mut |path, _, node| {
            if path.is_empty() {
                return;
            }
            out.push(match node.scalar() {
                Some(s) => format!("{} = {}", path.join("."), s.text),
                None => path.join("."),
            });
        });
        out
    }

    #[test]
    fn json_values_and_positions() {
        let d = doc("a.json", "{\n  \"a\": {\"b\": [1, \"x\\ny\", true, null]},\n  \"k\": -1.5\n}\n");
        assert_eq!(dump(&d), ["a", "a.b", "a.b.0 = 1", "a.b.1 = x\ny", "a.b.2 = true", "a.b.3 = null", "k = -1.5"]);
        let mut at_k = None;
        d.walk(&mut |path, at, _| {
            if path == ["k"] {
                at_k = Some(at);
            }
        });
        assert_eq!(at_k, Some((3, 3)));
    }

    #[test]
    fn json_comments_are_tolerated() {
        assert_eq!(dump(&doc("a.json", "// note\n{\"a\": 1}\n")), ["a = 1"]);
    }

    #[test]
    fn yaml_blocks_flows_scalars_and_documents() {
        let text = "a: &x\n  b: [1, \"two\", 'it''s']\n  c: {d: e}\nlist:\n  - name: x\n    run: |\n      echo hi\n      echo\n  - plain\nref: *x\nempty:\n---\nb: 1\n";
        let d = doc("a.yaml", text);
        assert_eq!(d.roots.len(), 2);
        assert_eq!(
            dump(&d),
            [
                "a", "a.b", "a.b.0 = 1", "a.b.1 = two", "a.b.2 = it's", "a.c", "a.c.d = e", "list", "list.0", "list.0.name = x",
                "list.0.run = echo hi\necho", "list.1 = plain", "ref = *x", "empty = null", "b = 1"
            ]
        );
    }

    #[test]
    fn toml_quoted_keys_in_headers_and_duplicate_keys() {
        let d = doc("a.toml", "[\"a.b\".c]\nx = 1\n['single']\ny = 2\n[t]\nk = 1\nk = 2\n[[arr]]\n[[arr]]\nv = 3\n");
        assert_eq!(dump(&d), ["a.b", "a.b.c", "a.b.c.x = 1", "single", "single.y = 2", "t", "t.k = 1", "t.k = 2", "arr", "arr.0", "arr.1", "arr.1.v = 3"]);
        // A quoted segment in a path pattern finds the dotted key.
        let found = d.registry(&PathPattern::parse("\"a.b\".c.x").unwrap(), Use::Value);
        assert_eq!(found.into_keys().collect::<Vec<_>>(), ["1"]);
    }

    #[test]
    fn json_duplicate_keys_are_kept() {
        assert_eq!(dump(&doc("a.json", "{\"a\": 1, \"a\": 2}")), ["a = 1", "a = 2"]);
    }

    #[test]
    fn empty_files_have_no_roots() {
        assert!(doc("a.json", "").roots.is_empty());
        assert!(doc("a.yaml", "").roots.is_empty());
        assert_eq!(doc("a.toml", "").roots.len(), 1);
    }

    fn matches(pattern: &str, path: &str) -> bool {
        let path: Vec<String> = path.split('.').map(String::from).collect();
        PathPattern::parse(pattern).unwrap().matches(&path)
    }

    #[test]
    fn path_patterns() {
        assert!(matches("scripts.postinstall", "scripts.postinstall"));
        assert!(!matches("scripts.postinstall", "x.scripts.postinstall"));
        assert!(matches("jobs.*.runs-on", "jobs.build.runs-on"));
        assert!(!matches("jobs.*.runs-on", "jobs.build.steps.runs-on"));
        assert!(matches("jobs.*.steps.*.uses", "jobs.build.steps.0.uses"));
        assert!(matches("**.version", "version"));
        assert!(matches("**.version", "a.b.version"));
        assert!(!matches("**.version", "a.b.versions"));
        assert!(matches("dependencies.@types/*", "dependencies.@types/node"));
        assert!(matches("dependencies.*", "dependencies.react"));
        assert!(!matches("dependencies.*", "dependencies"));
        assert!(PathPattern::parse("\"a.b\".c").unwrap().matches(&["a.b".into(), "c".into()]));
        assert!(PathPattern::parse("a..b").is_err());
        assert!(PathPattern::parse("\"a.b").is_err());
    }

    #[test]
    fn split_last_names_the_container_and_the_key() {
        let (parent, key) = PathPattern::parse("jobs.*.timeout-minutes").unwrap().split_last().unwrap();
        assert_eq!(key, "timeout-minutes");
        assert!(parent.matches(&["jobs".into(), "build".into()]));
        assert!(!parent.is_literal());
        assert!(PathPattern::parse("jobs.*").unwrap().split_last().is_none());
    }

    #[test]
    fn placeholders_in_messages() {
        let found = |t: &str| placeholders(t).into_iter().collect::<Vec<_>>();
        assert_eq!(found("Hello {{name}}, you have {{ count }}"), ["count", "name"]);
        assert_eq!(found("{count, plural, one {# item} other {# items}} for {user}"), ["count", "user"]);
        assert_eq!(found("No placeholders, just { some braces } and {}"), Vec::<String>::new());
        assert_eq!(found("${version}"), ["version"]);
    }

    #[test]
    fn registries_and_leaves() {
        let d = doc("a.json", "{\"codes\": {\"a_failed.x\": 1, \"b_failed.y\": 2}, \"list\": [\"p\", \"q\"]}");
        let names = |use_| d.registry(&PathPattern::parse("codes.*").unwrap(), use_).into_keys().collect::<Vec<_>>();
        assert_eq!(names(Use::Key), ["a_failed.x", "b_failed.y"]);
        assert_eq!(names(Use::Value), ["1", "2"]);
        assert_eq!(names(Use::Path), ["codes.a_failed.x", "codes.b_failed.y"]);
        let values = d.registry(&PathPattern::parse("list.*").unwrap(), Use::Value);
        assert_eq!(values.into_keys().collect::<Vec<_>>(), ["p", "q"]);
        let leaves: Vec<String> = d.leaves().keys().map(|k| dotted(k)).collect();
        assert_eq!(leaves, ["codes.\"a_failed.x\"", "codes.\"b_failed.y\"", "list"]);
        assert_eq!(d.deepest_existing(&["codes".into(), "zzz".into(), "q".into()]), (1, 2));
    }

    #[test]
    fn a_dotted_key_is_not_a_nested_key() {
        let flat = doc("a.json", "{\"a.b\": 1}");
        let nested = doc("b.json", "{\"a\": {\"b\": 1}}");
        assert_ne!(flat.leaves().keys().collect::<Vec<_>>(), nested.leaves().keys().collect::<Vec<_>>());
        assert_eq!(dotted(&["a.b".into(), "c".into()]), "\"a.b\".c");
        // The missing key is reported at the deepest key that really exists.
        assert_eq!(nested.deepest_existing(&["a.b".into()]), (1, 1));
    }
}
