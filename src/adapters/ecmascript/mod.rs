//! JavaScript and TypeScript share one adapter: the TypeScript grammar is a superset.

mod tsconfig;

use tree_sitter::Node;

use super::{Adapter, Assignment, ClassMembers, Comparison, Export, ImportRef, Member, Property, first_named_child, last_segment, relative_path, text};
use crate::check::Project;

pub use tsconfig::TsConfigs;

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

    fn class_family(&self) -> Option<&'static str> {
        Some("ecmascript")
    }

    /// `class A extends B`; in TypeScript the base sits in an `extends` clause and may carry type arguments.
    fn class_bases(&self, node: Node, src: &str) -> Option<(String, Vec<String>)> {
        if !matches!(node.kind(), "class_declaration" | "abstract_class_declaration") {
            return None;
        }
        let name = text(node.child_by_field_name("name")?, src).to_string();
        let mut bases = Vec::new();
        let mut cursor = node.walk();
        for heritage in node.named_children(&mut cursor).filter(|c| c.kind() == "class_heritage") {
            let mut inner = heritage.walk();
            for child in heritage.named_children(&mut inner) {
                let base = match child.kind() {
                    "extends_clause" => child.child_by_field_name("value"),
                    "implements_clause" => None,
                    _ => Some(child),
                };
                if let Some(dotted) = base.and_then(|b| es_dotted_name(b, src)) {
                    bases.push(last_segment(&dotted).to_string());
                }
            }
        }
        Some((name, bases))
    }

    /// Methods (including `constructor`) and fields declared in the class body.
    fn class_members<'t>(&self, node: Node<'t>, src: &str) -> Option<ClassMembers<'t>> {
        if !matches!(node.kind(), "class_declaration" | "abstract_class_declaration") {
            return None;
        }
        let mut members = ClassMembers { methods: Vec::new(), fields: Vec::new() };
        let Some(body) = node.child_by_field_name("body") else { return Some(members) };
        let mut cursor = body.walk();
        for member in body.named_children(&mut cursor) {
            match member.kind() {
                "method_definition" | "abstract_method_signature" => {
                    members.methods.extend(member.child_by_field_name("name").and_then(|n| Self::key_name(n, src)));
                }
                // `x: T` and `x = 1` in TypeScript; plain `x = 1` is a `field_definition` in JavaScript.
                "public_field_definition" | "field_definition" => {
                    let key = member.child_by_field_name("name").or_else(|| member.child_by_field_name("property"));
                    if let Some(key) = key
                        && let Some(name) = Self::key_name(key, src)
                    {
                        members.fields.push(Member { name, node: key });
                    }
                }
                _ => {}
            }
        }
        Some(members)
    }

    fn assignment<'t>(&self, node: Node<'t>, src: &str) -> Option<Assignment<'t>> {
        let (augmented, declaration) = match node.kind() {
            "assignment_expression" => (false, false),
            "augmented_assignment_expression" => (true, false),
            "variable_declarator" => (false, true),
            _ => return None,
        };
        let target = node.child_by_field_name(if declaration { "name" } else { "left" })?;
        let value = node.child_by_field_name(if declaration { "value" } else { "right" });
        if declaration && value.is_none() {
            return None;
        }
        let target = match target.kind() {
            "identifier" | "member_expression" => es_dotted_name(target, src).unwrap_or_else(|| "?".to_string()),
            "subscript_expression" => format!("{}[]", es_dotted_name(target.child_by_field_name("object")?, src).unwrap_or_else(|| "?".to_string())),
            _ => return None,
        };
        Some(Assignment { target, augmented, declaration, value })
    }

    fn comparison<'t>(&self, node: Node<'t>, src: &str) -> Option<Comparison<'t>> {
        if node.kind() != "binary_expression" {
            return None;
        }
        let operator = text(node.child_by_field_name("operator")?, src);
        if !matches!(operator, "==" | "===" | "!=" | "!==" | "<" | "<=" | ">" | ">=" | "instanceof" | "in") {
            return None;
        }
        Some(Comparison { operators: vec![operator.to_string()], operands: vec![node.child_by_field_name("left")?, node.child_by_field_name("right")?] })
    }

    /// `{ key: value }`; computed keys and spreads are left out.
    fn dict_entries<'t>(&self, node: Node<'t>, src: &str) -> Option<Vec<(String, Node<'t>)>> {
        if node.kind() != "object" {
            return None;
        }
        let mut cursor = node.walk();
        let entries = node
            .named_children(&mut cursor)
            .filter_map(|member| match member.kind() {
                "pair" => Some((Self::key_name(member.child_by_field_name("key")?, src)?, member.child_by_field_name("key")?)),
                "shorthand_property_identifier" => Some((text(member, src).to_string(), member)),
                _ => None,
            })
            .collect();
        Some(entries)
    }

    /// Function declarations, methods, and `const f = () => ...` (named after the variable).
    fn function_name(&self, node: Node, src: &str) -> Option<String> {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" | "method_definition" => {
                Self::key_name(node.child_by_field_name("name")?, src)
            }
            "variable_declarator" => {
                let value = node.child_by_field_name("value")?;
                if !matches!(value.kind(), "arrow_function" | "function_expression" | "function") {
                    return None;
                }
                Some(text(node.child_by_field_name("name")?, src).to_string())
            }
            _ => None,
        }
    }

    /// `export const a = 1, b = 2`, `export function F() {}`, `export default F;`, `export { A, B as C }` and
    /// `export * from "m"`. Interfaces, type aliases and `export type { T }` are types, not values, and are left out.
    fn exports<'t>(&self, node: Node<'t>, src: &str) -> Vec<Export<'t>> {
        if node.kind() != "export_statement" {
            return Vec::new();
        }
        let mut cursor = node.walk();
        let tokens: Vec<&str> = node.children(&mut cursor).filter(|c| !c.is_named()).map(|c| c.kind()).collect();
        if tokens.contains(&"type") {
            return Vec::new();
        }
        let default = tokens.contains(&"default");
        let export = |name: String, default: bool, declared: bool| Export { name, default, declared, node };
        if let Some(declaration) = node.child_by_field_name("declaration") {
            let names = match declaration.kind() {
                "interface_declaration" | "type_alias_declaration" => return Vec::new(),
                "lexical_declaration" | "variable_declaration" => {
                    let mut cursor = declaration.walk();
                    let names: Vec<String> = declaration
                        .named_children(&mut cursor)
                        .filter(|d| d.kind() == "variable_declarator")
                        .filter_map(|d| d.child_by_field_name("name"))
                        .filter(|n| n.kind() == "identifier")
                        .map(|n| text(n, src).to_string())
                        .collect();
                    names
                }
                _ => vec![declaration.child_by_field_name("name").map(|n| text(n, src).to_string()).unwrap_or_default()],
            };
            return names.into_iter().map(|name| export(name, default, true)).collect();
        }
        if let Some(value) = node.child_by_field_name("value") {
            let anonymous = matches!(value.kind(), "arrow_function" | "function_expression" | "function" | "generator_function" | "class");
            return vec![if anonymous { export(String::new(), true, true) } else { export(text(value, src).to_string(), true, false) }];
        }
        let mut cursor = node.walk();
        let mut out = Vec::new();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "export_clause" => {
                    let mut inner = child.walk();
                    for specifier in child.named_children(&mut inner).filter(|c| c.kind() == "export_specifier") {
                        let Some(name) = specifier.child_by_field_name("name").map(|n| text(n, src).to_string()) else { continue };
                        match specifier.child_by_field_name("alias").map(|a| text(a, src)) {
                            Some("default") => out.push(export(name, true, false)),
                            Some(alias) => out.push(export(alias.to_string(), false, false)),
                            None => out.push(export(name, false, false)),
                        }
                    }
                }
                "namespace_export" => out.extend(first_named_child(child).map(|n| export(text(n, src).to_string(), false, false))),
                _ => {}
            }
        }
        if out.is_empty() && tokens.contains(&"*") {
            out.push(export("*".to_string(), false, false));
        }
        out
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
    } else if let Some(candidates) = project.resolvers.ts.resolve(specifier, file_path) {
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
