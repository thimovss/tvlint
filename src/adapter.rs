//! Language adapters: the only place that knows a language's tree-sitter node kinds.
//!
//! Rules are evaluated against a few language-neutral questions ("is this node a raise,
//! and of what?", "is this node a call, and to what?"). Each language answers them from
//! its own syntax tree, so adding a language means adding an adapter, not touching the engine.

use tree_sitter::Node;

use crate::config::Language;

/// An object-literal property: `key: value`, a shorthand `{ key }`, or a method `key() {}`.
pub struct Property<'t> {
    pub key: String,
    pub value: Option<Node<'t>>,
}

pub trait Adapter: Sync {
    /// If `node` raises/throws an exception, the name of what is raised.
    fn raised_name(&self, node: Node, src: &str) -> Option<String>;

    /// If `node` is a call, the callee as written, e.g. `session.exec().one`.
    fn call_name(&self, node: Node, src: &str) -> Option<String>;

    /// If `node` is an exception handler, the names of the types it catches.
    fn caught_types(&self, _node: Node, _src: &str) -> Option<Vec<String>> {
        None
    }

    /// If `node` imports a module, the specifier as written, e.g. `@/api` or `react`.
    fn import_specifier(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// If `node` opens a JSX element, its name as written, e.g. `button` or `Foo.Bar`.
    fn element_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// If `node` is a property of an object literal, its key and value.
    fn object_property<'t>(&self, _node: Node<'t>, _src: &str) -> Option<Property<'t>> {
        None
    }

    /// If `node` is passed directly as an argument of a call, the callee as written.
    fn argument_of_call(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// Whether `node` is a string literal (a template literal without substitutions counts).
    fn is_string_literal(&self, _node: Node, _src: &str) -> bool {
        false
    }

    /// If `node` is an array literal, its elements.
    fn array_elements<'t>(&self, _node: Node<'t>) -> Option<Vec<Node<'t>>> {
        None
    }

    /// Whether this language contributes to the class hierarchy used by `forbid_catch`.
    fn indexes_classes(&self) -> bool {
        false
    }

    /// If `node` defines a class, its name and the simple names of its base classes.
    fn class_bases(&self, _node: Node, _src: &str) -> Option<(String, Vec<String>)> {
        None
    }
}

pub fn for_language(language: Language) -> &'static dyn Adapter {
    match language {
        Language::Python => &Python,
        Language::JavaScript | Language::TypeScript => &EcmaScript,
    }
}

pub fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    &src[node.byte_range()]
}

fn first_named_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|n| n.kind() != "comment")
}

// ---------------------------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------------------------

struct Python;

impl Adapter for Python {
    fn raised_name(&self, node: Node, src: &str) -> Option<String> {
        if node.kind() != "raise_statement" {
            return None;
        }
        // `raise Foo(...) from cause`: the exception is the first expression that is not the cause.
        let cause = node.child_by_field_name("cause").map(|n| n.id());
        let mut cursor = node.walk();
        let expr = node
            .named_children(&mut cursor)
            .find(|n| n.kind() != "comment" && Some(n.id()) != cause)?;
        let target = if expr.kind() == "call" { expr.child_by_field_name("function")? } else { expr };
        dotted_name(target, src)
    }

    fn call_name(&self, node: Node, src: &str) -> Option<String> {
        if node.kind() != "call" {
            return None;
        }
        Some(callee_name(node.child_by_field_name("function")?, src))
    }

    fn caught_types(&self, node: Node, src: &str) -> Option<Vec<String>> {
        if !matches!(node.kind(), "except_clause" | "except_group_clause") {
            return None;
        }
        let mut cursor = node.walk();
        let Some(mut first) = node
            .named_children(&mut cursor)
            .find(|n| !matches!(n.kind(), "block" | "comment"))
        else {
            return Some(Vec::new());
        };
        if first.kind() == "as_pattern" {
            let Some(inner) = first.named_child(0) else { return Some(Vec::new()) };
            first = inner;
        }
        let mut names = Vec::new();
        if matches!(first.kind(), "tuple" | "parenthesized_expression" | "expression_list") {
            let mut cursor = first.walk();
            for item in first.named_children(&mut cursor) {
                names.extend(dotted_name(item, src));
            }
        } else {
            names.extend(dotted_name(first, src));
        }
        Some(names)
    }

    fn indexes_classes(&self) -> bool {
        true
    }

    fn class_bases(&self, node: Node, src: &str) -> Option<(String, Vec<String>)> {
        if node.kind() != "class_definition" {
            return None;
        }
        let name = text(node.child_by_field_name("name")?, src).to_string();
        let mut bases = Vec::new();
        if let Some(supers) = node.child_by_field_name("superclasses") {
            let mut cursor = supers.walk();
            for base in supers.named_children(&mut cursor) {
                if let Some(dotted) = dotted_name(base, src) {
                    bases.push(last_segment(&dotted).to_string());
                }
            }
        }
        Some((name, bases))
    }
}

/// `a.b.c` for plain identifier/attribute chains, otherwise `None`.
fn dotted_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, src).to_string()),
        "attribute" => {
            let object = dotted_name(node.child_by_field_name("object")?, src)?;
            let attr = text(node.child_by_field_name("attribute")?, src);
            Some(format!("{object}.{attr}"))
        }
        _ => None,
    }
}

/// Like `dotted_name` but total: calls render as `f()` and anything else as `?`,
/// so `session.exec(q).one` becomes `session.exec().one`.
fn callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" => text(node, src).to_string(),
        "attribute" => {
            let object = node
                .child_by_field_name("object")
                .map_or_else(|| "?".to_string(), |o| callee_name(o, src));
            let attr = node.child_by_field_name("attribute").map_or("?", |a| text(a, src));
            format!("{object}.{attr}")
        }
        "call" => {
            let function = node
                .child_by_field_name("function")
                .map_or_else(|| "?".to_string(), |f| callee_name(f, src));
            format!("{function}()")
        }
        _ => "?".to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// JavaScript and TypeScript share one adapter: the TypeScript grammar is a superset.
// ---------------------------------------------------------------------------------------------

struct EcmaScript;

impl Adapter for EcmaScript {
    fn raised_name(&self, node: Node, src: &str) -> Option<String> {
        if node.kind() != "throw_statement" {
            return None;
        }
        let expr = first_named_child(node)?;
        // `throw new Foo(...)` and `throw Foo(...)` both name Foo; `throw err` names err.
        let target = match expr.kind() {
            "new_expression" => expr.child_by_field_name("constructor")?,
            "call_expression" => expr.child_by_field_name("function")?,
            _ => expr,
        };
        es_dotted_name(target, src)
    }

    /// `new Foo(...)` counts as a call to `Foo`.
    fn call_name(&self, node: Node, src: &str) -> Option<String> {
        match node.kind() {
            "call_expression" => Some(es_callee_name(node.child_by_field_name("function")?, src)),
            "new_expression" => Some(es_callee_name(node.child_by_field_name("constructor")?, src)),
            _ => None,
        }
    }

    /// `import x from "m"`, `export * from "m"`, `import("m")`, `require("m")` and `import x = require("m")`.
    fn import_specifier(&self, node: Node, src: &str) -> Option<String> {
        match node.kind() {
            "import_statement" | "export_statement" => {
                Self::string_value(node.child_by_field_name("source")?, src)
            }
            "import_require_clause" => {
                let mut cursor = node.walk();
                let source = node.named_children(&mut cursor).find(|c| c.kind() == "string")?;
                Self::string_value(source, src)
            }
            "call_expression" => {
                let function = node.child_by_field_name("function")?;
                let is_import = function.kind() == "import"
                    || (function.kind() == "identifier" && text(function, src) == "require");
                if !is_import {
                    return None;
                }
                let arguments = node.child_by_field_name("arguments")?;
                let first = first_named_child(arguments)?;
                Self::string_value(first, src)
            }
            _ => None,
        }
    }

    fn element_name(&self, node: Node, src: &str) -> Option<String> {
        if !matches!(node.kind(), "jsx_opening_element" | "jsx_self_closing_element") {
            return None;
        }
        Some(text(node.child_by_field_name("name")?, src).to_string())
    }

    /// Only object literals: JSX attributes such as `onError={...}` are not properties.
    fn object_property<'t>(&self, node: Node<'t>, src: &str) -> Option<Property<'t>> {
        match node.kind() {
            "pair" => Some(Property {
                key: Self::key_name(node.child_by_field_name("key")?, src)?,
                value: node.child_by_field_name("value"),
            }),
            "shorthand_property_identifier" => Some(Property { key: text(node, src).to_string(), value: None }),
            "method_definition" if node.parent().is_some_and(|p| p.kind() == "object") => Some(Property {
                key: Self::key_name(node.child_by_field_name("name")?, src)?,
                value: None,
            }),
            _ => None,
        }
    }

    fn argument_of_call(&self, node: Node, src: &str) -> Option<String> {
        let arguments = node.parent().filter(|p| p.kind() == "arguments")?;
        self.call_name(arguments.parent()?, src)
    }

    fn is_string_literal(&self, node: Node, src: &str) -> bool {
        Self::string_value(node, src).is_some()
    }

    fn array_elements<'t>(&self, node: Node<'t>) -> Option<Vec<Node<'t>>> {
        // `[...] as const` and `([...])` are still the array.
        let mut node = node;
        while matches!(node.kind(), "as_expression" | "satisfies_expression" | "parenthesized_expression") {
            node = first_named_child(node)?;
        }
        if node.kind() != "array" {
            return None;
        }
        let mut cursor = node.walk();
        Some(node.named_children(&mut cursor).filter(|c| c.kind() != "comment").collect())
    }
}

impl EcmaScript {
    /// The text between the quotes of a plain string, or of a template literal without `${}`.
    fn string_value(node: Node, src: &str) -> Option<String> {
        match node.kind() {
            "string" => {
                let raw = text(node, src);
                raw.get(1..raw.len().checked_sub(1)?).map(str::to_string)
            }
            "template_string" if !has_substitution(node) => {
                let raw = text(node, src);
                raw.get(1..raw.len().checked_sub(1)?).map(str::to_string)
            }
            _ => None,
        }
    }

    /// `foo`, `"foo"` and `'foo'` all name the key `foo`.
    fn key_name(node: Node, src: &str) -> Option<String> {
        match node.kind() {
            "property_identifier" | "identifier" | "private_property_identifier" | "number" => {
                Some(text(node, src).to_string())
            }
            "string" => Self::string_value(node, src),
            _ => None,
        }
    }
}

fn has_substitution(node: Node) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).any(|c| c.kind() == "template_substitution")
}

fn es_dotted_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "this" => Some(text(node, src).to_string()),
        "member_expression" => {
            let object = es_dotted_name(node.child_by_field_name("object")?, src)?;
            let property = text(node.child_by_field_name("property")?, src);
            Some(format!("{object}.{property}"))
        }
        _ => None,
    }
}

/// Total version of `es_dotted_name`: `session.exec(q).one` becomes `session.exec().one`,
/// `new Date().getTime` becomes `new Date().getTime`, and anything unrecognised is `?`.
fn es_callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" | "this" | "super" | "import" => text(node, src).to_string(),
        "member_expression" => {
            let object = node
                .child_by_field_name("object")
                .map_or_else(|| "?".to_string(), |o| es_callee_name(o, src));
            let property = node.child_by_field_name("property").map_or("?", |p| text(p, src));
            format!("{object}.{property}")
        }
        "call_expression" => {
            let function = node
                .child_by_field_name("function")
                .map_or_else(|| "?".to_string(), |f| es_callee_name(f, src));
            format!("{function}()")
        }
        "new_expression" => {
            let constructor = node
                .child_by_field_name("constructor")
                .map_or_else(|| "?".to_string(), |c| es_callee_name(c, src));
            format!("new {constructor}()")
        }
        // Wrappers that do not change what is being called: (x), x!, x as T, x satisfies T.
        "parenthesized_expression" | "non_null_expression" | "as_expression" | "satisfies_expression" => {
            first_named_child(node).map_or_else(|| "?".to_string(), |inner| es_callee_name(inner, src))
        }
        _ => "?".to_string(),
    }
}
