//! JavaScript and TypeScript share one adapter: the TypeScript grammar is a superset.

use tree_sitter::Node;

use super::{Adapter, ImportRef, Property, first_named_child, relative_path, text};
use crate::check::Project;

pub(super) struct EcmaScript;

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

    fn imports(&self, node: Node, src: &str, file_path: &str, project: &Project) -> Vec<ImportRef> {
        let Some(specifier) = Self::import_specifier(node, src) else { return Vec::new() };
        let resolved = es_resolve(&specifier, file_path, project);
        vec![ImportRef { specifier, resolved }]
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

    fn call_arguments<'t>(&self, node: Node<'t>) -> Option<Vec<Node<'t>>> {
        if !matches!(node.kind(), "call_expression" | "new_expression") {
            return None;
        }
        let arguments = node.child_by_field_name("arguments")?;
        let mut cursor = arguments.walk();
        let mut positional = Vec::new();
        for argument in arguments.named_children(&mut cursor).filter(|n| n.kind() != "comment") {
            if argument.kind() == "spread_element" {
                break;
            }
            positional.push(argument);
        }
        Some(positional)
    }

    fn string_value(&self, node: Node, src: &str) -> Option<String> {
        Self::string_value(node, src)
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
    /// `import x from "m"`, `export * from "m"`, `import("m")`, `require("m")` and `import x = require("m")`.
    fn import_specifier(node: Node, src: &str) -> Option<String> {
        match node.kind() {
            "import_statement" | "export_statement" => Self::string_value(node.child_by_field_name("source")?, src),
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
                Self::string_value(first_named_child(arguments)?, src)
            }
            _ => None,
        }
    }

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

const EXTENSIONS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts", ".json"];

/// The project path an import points at, or the specifier itself for a package.
/// `./x` and `../x` are resolved against the importing file, aliases are expanded, and a path
/// without extension is matched to the file that exists (`x.ts`, `x/index.tsx`, ...).
fn es_resolve(specifier: &str, file_path: &str, project: &Project) -> String {
    // Several candidates when a tsconfig alias lists fallbacks; the first one that exists wins.
    let candidates: Vec<String> = if specifier.starts_with("./") || specifier.starts_with("../") {
        match relative_path(specifier, file_path) {
            Some(path) => vec![path],
            None => return specifier.to_string(), // leaves the project
        }
    } else if let Some((prefix, target)) = project
        .aliases
        .iter()
        .filter(|(prefix, _)| specifier.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
    {
        // The aliases written in tvlint.yaml win over the ones read from tsconfig.
        vec![format!("{target}{}", &specifier[prefix.len()..])]
    } else if let Some(candidates) = project.modules.ts.resolve(specifier, file_path) {
        candidates
    } else {
        return specifier.to_string();
    };
    let existing = |path: &str| -> Option<String> {
        if project.paths.contains(path) {
            return Some(path.to_string());
        }
        EXTENSIONS
            .iter()
            .flat_map(|ext| [format!("{path}{ext}"), format!("{path}/index{ext}")])
            .find(|candidate| project.paths.contains(candidate))
    };
    let candidates: Vec<String> = candidates.into_iter().map(|c| c.trim_end_matches('/').to_string()).collect();
    candidates.iter().find_map(|c| existing(c)).unwrap_or_else(|| candidates[0].clone())
}
