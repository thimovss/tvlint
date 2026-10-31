//! Language adapters: the only place that knows a language's tree-sitter node kinds.
//!
//! Rules are evaluated against a few language-neutral questions ("is this node a raise,
//! and of what?", "is this node a call, and to what?"). Each language answers them from
//! its own syntax tree, so adding a language means adding an adapter, not touching the engine.

use tree_sitter::Node;

use crate::check::Project;
use crate::config::Language;

/// One module imported by a statement: as written, and as the project path it leads to.
pub struct ImportRef {
    pub specifier: String,
    pub resolved: String,
}

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

    /// The modules `node` imports. A statement may import several (`use a::{b, c}`).
    fn imports(&self, _node: Node, _src: &str, _file_path: &str, _project: &Project) -> Vec<ImportRef> {
        Vec::new()
    }

    /// If `node` opens a JSX or HTML element, its name, e.g. `button` or `Foo.Bar`.
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

    /// If `node` is a call, its leading positional arguments, up to the first spread or keyword argument.
    fn call_arguments<'t>(&self, _node: Node<'t>) -> Option<Vec<Node<'t>>> {
        None
    }

    /// The text of a plain string literal (no interpolation), without its quotes.
    fn string_value(&self, _node: Node, _src: &str) -> Option<String> {
        None
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
        Language::Rust => &Rust,
        Language::Go => &Go,
        Language::Html => &Html,
        Language::Shell => &Shell,
        Language::Css => &Css,
        Language::Json | Language::Yaml | Language::Toml => &Data,
    }
}

/// Data files are read through `data::Doc`, not through syntax questions: nothing here applies.
struct Data;

impl Adapter for Data {
    fn raised_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    fn call_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
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

    fn call_arguments<'t>(&self, node: Node<'t>) -> Option<Vec<Node<'t>>> {
        if node.kind() != "call" {
            return None;
        }
        let arguments = node.child_by_field_name("arguments")?;
        let mut cursor = arguments.walk();
        let mut positional = Vec::new();
        for argument in arguments.named_children(&mut cursor).filter(|n| n.kind() != "comment") {
            if matches!(argument.kind(), "keyword_argument" | "list_splat" | "dictionary_splat") {
                break;
            }
            positional.push(argument);
        }
        Some(positional)
    }

    fn string_value(&self, node: Node, src: &str) -> Option<String> {
        if node.kind() != "string" {
            return None;
        }
        let raw = text(node, src);
        let quote = raw.find(['"', '\''])?;
        // f-strings interpolate; bytes are not text; raw strings keep their backslashes (fine for ids).
        if raw[..quote].contains(['f', 'F', 'b', 'B']) {
            return None;
        }
        let rest = &raw[quote..];
        let quotes = if rest.starts_with("\"\"\"") || rest.starts_with("'''") { 3 } else { 1 };
        rest.get(quotes..rest.len().checked_sub(quotes)?).map(str::to_string)
    }

    /// `import a.b as c`, `from a.b import c, d`, `from . import x` and `from ..p import *`.
    /// A `from` import names `module.name` for each name, so `from a import b` is `a.b`.
    fn imports(&self, node: Node, src: &str, file_path: &str, project: &Project) -> Vec<ImportRef> {
        let mut cursor = node.walk();
        let specifiers: Vec<String> = match node.kind() {
            "import_statement" => node
                .children_by_field_name("name", &mut cursor)
                .filter_map(|name| py_imported_name(name, src))
                .collect(),
            "import_from_statement" => {
                let Some(module) = node.child_by_field_name("module_name") else { return Vec::new() };
                let module = text(module, src).split_whitespace().collect::<String>();
                let sep = if module.ends_with('.') { "" } else { "." };
                let mut names: Vec<String> = node
                    .children_by_field_name("name", &mut cursor)
                    .filter_map(|name| py_imported_name(name, src))
                    .collect();
                if node.named_children(&mut node.walk()).any(|c| c.kind() == "wildcard_import") {
                    names.push("*".to_string());
                }
                if names.is_empty() {
                    vec![module]
                } else {
                    names.into_iter().map(|name| format!("{module}{sep}{name}")).collect()
                }
            }
            _ => return Vec::new(),
        };
        specifiers
            .into_iter()
            .map(|specifier| {
                let resolved = py_resolve(&specifier, file_path, project);
                ImportRef { specifier, resolved }
            })
            .collect()
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

/// The dotted name an `import` or `from ... import` item brings in, without its `as` alias.
fn py_imported_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "dotted_name" => Some(text(node, src).split_whitespace().collect()),
        "aliased_import" => py_imported_name(node.child_by_field_name("name")?, src),
        _ => None,
    }
}

/// The project file a Python import leads to, or the specifier itself for a package from outside.
/// Absolute imports start at the source roots of the nearest `pyproject.toml`; relative ones
/// (`.x`, `..p.y`) at the importing file's package. `a.b.c` also finds `a/b.py` when `c` is a name in it.
fn py_resolve(specifier: &str, file_path: &str, project: &Project) -> String {
    let dots = specifier.chars().take_while(|c| *c == '.').count();
    let parts: Vec<&str> = specifier[dots..].split('.').filter(|p| !p.is_empty() && *p != "*").collect();
    let bases: Vec<String> = if dots == 0 {
        project.modules.py.roots_for(file_path).to_vec()
    } else {
        let mut dir: Vec<&str> = file_path.split('/').collect();
        dir.pop(); // the file name
        for _ in 1..dots {
            if dir.pop().is_none() {
                return specifier.to_string(); // above the project
            }
        }
        vec![dir.join("/")]
    };
    for base in &bases {
        for length in (1..=parts.len()).rev() {
            let module = crate::modules::join(base, &parts[..length].join("/"));
            for candidate in [format!("{module}.py"), format!("{module}/__init__.py")] {
                if project.paths.contains(&candidate) {
                    return candidate;
                }
            }
        }
    }
    specifier.to_string()
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

/// `./x` or `../x` resolved against the directory of `file_path`; `None` if it leaves the project.
fn relative_path(specifier: &str, file_path: &str) -> Option<String> {
    let mut parts: Vec<&str> = file_path.split('/').collect();
    parts.pop(); // the file name
    for segment in specifier.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
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

// ---------------------------------------------------------------------------------------------
// Rust. No exceptions: the checks are calls (functions, methods and macros) and `use` paths.
// ---------------------------------------------------------------------------------------------

struct Rust;

impl Adapter for Rust {
    fn raised_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// Functions (`std::fs::read`, `Vec::new`), methods (`x.unwrap`) and macros (`println!`).
    fn call_name(&self, node: Node, src: &str) -> Option<String> {
        match node.kind() {
            "call_expression" => Some(rs_callee_name(node.child_by_field_name("function")?, src)),
            "macro_invocation" => Some(format!("{}!", rs_callee_name(node.child_by_field_name("macro")?, src))),
            _ => None,
        }
    }

    fn call_arguments<'t>(&self, node: Node<'t>) -> Option<Vec<Node<'t>>> {
        if node.kind() != "call_expression" {
            return None;
        }
        let arguments = node.child_by_field_name("arguments")?;
        let mut cursor = arguments.walk();
        Some(arguments.named_children(&mut cursor).filter(|n| !n.kind().ends_with("comment")).collect())
    }

    fn string_value(&self, node: Node, src: &str) -> Option<String> {
        if node.kind() != "string_literal" {
            return None;
        }
        let raw = text(node, src);
        let inner = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"'))?;
        // Escapes would need decoding; an identifier-like literal never has any.
        (!inner.contains('\\')).then(|| inner.to_string())
    }

    /// `use a::b`, `use a::{b, c::d}`, `use a::*`, `use a as b`, and `extern crate a`.
    fn imports(&self, node: Node, src: &str, file_path: &str, _project: &Project) -> Vec<ImportRef> {
        let mut specifiers = Vec::new();
        match node.kind() {
            "use_declaration" => {
                if let Some(argument) = node.child_by_field_name("argument") {
                    rs_use_paths(argument, "", src, &mut specifiers);
                }
            }
            "extern_crate_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    specifiers.push(text(name, src).to_string());
                }
            }
            _ => return Vec::new(),
        }
        let module = rs_module_path(node, file_path, src);
        specifiers
            .into_iter()
            .map(|specifier| {
                let resolved = rs_resolve(&specifier, &module);
                ImportRef { specifier, resolved }
            })
            .collect()
    }
}

/// How a call is written: `Vec::new`, `x.unwrap`, `s.parse().unwrap`, `foo().bar`.
/// Generics, `?`, `.await`, `&` and casts are looked through; anything else is `?`.
fn rs_callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" | "self" | "crate" | "super" | "type_identifier" | "primitive_type" | "field_identifier" => {
            text(node, src).to_string()
        }
        "scoped_identifier" => {
            let path = node
                .child_by_field_name("path")
                .map_or_else(String::new, |p| format!("{}::", rs_callee_name(p, src)));
            let name = node.child_by_field_name("name").map_or("?", |n| text(n, src));
            format!("{path}{name}")
        }
        "field_expression" => {
            let value = node
                .child_by_field_name("value")
                .map_or_else(|| "?".to_string(), |v| rs_callee_name(v, src));
            let field = node.child_by_field_name("field").map_or("?", |f| text(f, src));
            format!("{value}.{field}")
        }
        "call_expression" => {
            let function = node
                .child_by_field_name("function")
                .map_or_else(|| "?".to_string(), |f| rs_callee_name(f, src));
            format!("{function}()")
        }
        "generic_function" => node
            .child_by_field_name("function")
            .map_or_else(|| "?".to_string(), |f| rs_callee_name(f, src)),
        "generic_type" => node
            .child_by_field_name("type")
            .map_or_else(|| "?".to_string(), |t| rs_callee_name(t, src)),
        "try_expression" | "parenthesized_expression" | "await_expression" | "reference_expression"
        | "type_cast_expression" => first_named_child(node).map_or_else(|| "?".to_string(), |n| rs_callee_name(n, src)),
        _ => "?".to_string(),
    }
}

/// Every path a `use` argument imports, written with `::`.
fn rs_use_paths(node: Node, prefix: &str, src: &str, out: &mut Vec<String>) {
    let join = |path: &str| match (prefix.is_empty(), path.is_empty()) {
        (_, true) => prefix.to_string(),
        (true, false) => path.to_string(),
        (false, false) => format!("{prefix}::{path}"),
    };
    match node.kind() {
        "identifier" | "crate" | "super" | "scoped_identifier" => out.push(join(&rs_path_text(node, src))),
        // `use a::{self, b}`: the module itself.
        "self" if !prefix.is_empty() => out.push(prefix.to_string()),
        "self" => out.push("self".to_string()),
        "use_as_clause" => {
            if let Some(path) = node.child_by_field_name("path") {
                rs_use_paths(path, prefix, src, out);
            }
        }
        "use_wildcard" => {
            let path = first_named_child(node).map_or_else(String::new, |p| rs_path_text(p, src));
            out.push(format!("{}::*", join(&path)).trim_start_matches("::").to_string());
        }
        "scoped_use_list" => {
            let path = node.child_by_field_name("path").map_or_else(String::new, |p| rs_path_text(p, src));
            let inner = join(&path);
            if let Some(list) = node.child_by_field_name("list") {
                rs_use_paths(list, &inner, src, out);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for item in node.named_children(&mut cursor).filter(|c| c.kind() != "comment") {
                rs_use_paths(item, prefix, src, out);
            }
        }
        _ => {}
    }
}

/// `a::b::c` without whitespace.
fn rs_path_text(node: Node, src: &str) -> String {
    match node.kind() {
        "scoped_identifier" => {
            let path = node.child_by_field_name("path").map_or_else(String::new, |p| format!("{}::", rs_path_text(p, src)));
            format!("{path}{}", node.child_by_field_name("name").map_or("", |n| text(n, src)))
        }
        _ => text(node, src).to_string(),
    }
}

/// The module a `use` sits in, as path segments below the crate root: the file's own module
/// (`src/a/b.rs` is `a::b`, `src/a/mod.rs` is `a`) plus any inline `mod x { ... }` around it.
fn rs_module_path(node: Node, file_path: &str, src: &str) -> Vec<String> {
    let parts: Vec<&str> = file_path.split('/').collect();
    let mut module: Vec<String> = match parts.iter().rposition(|p| *p == "src") {
        Some(src_dir) => {
            let mut below: Vec<&str> = parts[src_dir + 1..].to_vec();
            if let Some(file) = below.pop() {
                let name = file.strip_suffix(".rs").unwrap_or(file);
                let is_root = below.is_empty() && matches!(name, "lib" | "main");
                if name != "mod" && !is_root {
                    below.push(name);
                }
            }
            below.into_iter().map(str::to_string).collect()
        }
        None => Vec::new(),
    };
    let mut inline = Vec::new();
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if ancestor.kind() == "mod_item"
            && let Some(name) = ancestor.child_by_field_name("name")
        {
            inline.push(text(name, src).to_string());
        }
        current = ancestor.parent();
    }
    module.extend(inline.into_iter().rev());
    module
}

/// `self::x` and `super::x` become `crate::...` paths; everything else is left as written.
fn rs_resolve(specifier: &str, module: &[String]) -> String {
    let mut parts: Vec<&str> = specifier.split("::").collect();
    let mut base: Vec<&str> = module.iter().map(String::as_str).collect();
    match parts.first().copied() {
        Some("self") => {
            parts.remove(0);
        }
        Some("super") => {
            while parts.first() == Some(&"super") {
                parts.remove(0);
                if base.pop().is_none() {
                    return specifier.to_string(); // above the crate root
                }
            }
        }
        _ => return specifier.to_string(),
    }
    let mut resolved = vec!["crate"];
    resolved.extend(base);
    resolved.extend(parts);
    resolved.join("::")
}

// ---------------------------------------------------------------------------------------------
// Go. No exceptions either: the checks are calls (`fmt.Println`, `db.Query`) and import paths.
// ---------------------------------------------------------------------------------------------

struct Go;

impl Adapter for Go {
    fn raised_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// Functions (`os.Exit`), methods (`db.Query`) and builtins (`panic`).
    /// `Sort[int](x)` is parsed as a conversion to a generic type, but it is a call.
    fn call_name(&self, node: Node, src: &str) -> Option<String> {
        match node.kind() {
            "call_expression" => Some(go_callee_name(node.child_by_field_name("function")?, src)),
            "type_conversion_expression" => {
                let callee = node.child_by_field_name("type").filter(|t| t.kind() == "generic_type")?;
                Some(go_callee_name(callee, src))
            }
            _ => None,
        }
    }

    fn call_arguments<'t>(&self, node: Node<'t>) -> Option<Vec<Node<'t>>> {
        if node.kind() != "call_expression" {
            return None;
        }
        let arguments = node.child_by_field_name("arguments")?;
        let mut cursor = arguments.walk();
        let mut positional = Vec::new();
        for argument in arguments.named_children(&mut cursor).filter(|n| n.kind() != "comment") {
            if argument.kind() == "variadic_argument" {
                break;
            }
            positional.push(argument);
        }
        Some(positional)
    }

    fn string_value(&self, node: Node, src: &str) -> Option<String> {
        let raw = text(node, src);
        let inner = raw.get(1..raw.len().checked_sub(1)?)?;
        match node.kind() {
            // Escapes would need decoding; an identifier-like literal never has any.
            "interpreted_string_literal" => (!inner.contains('\\')).then(|| inner.to_string()),
            "raw_string_literal" => Some(inner.to_string()),
            _ => None,
        }
    }

    /// Every `import` spec: `"fmt"`, `db "app/db"`, `. "app/db"` and `_ "app/db"`.
    /// A path inside one of the project's modules (see `go.mod`) resolves to its project directory.
    fn imports(&self, node: Node, src: &str, _file_path: &str, project: &Project) -> Vec<ImportRef> {
        if node.kind() != "import_spec" {
            return Vec::new();
        }
        let Some(path) = node.child_by_field_name("path") else { return Vec::new() };
        match self.string_value(path, src) {
            Some(specifier) => {
                let resolved = project.modules.go.resolve(&specifier).unwrap_or_else(|| specifier.clone());
                vec![ImportRef { specifier, resolved }]
            }
            None => Vec::new(),
        }
    }
}

/// How a call is written: `fmt.Println`, `db.Query`, `s.Get().Do`, `panic`.
/// Parentheses and generic instantiation (`f[int](x)`) are looked through; anything else is `?`.
fn go_callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" | "field_identifier" | "package_identifier" | "type_identifier" => text(node, src).to_string(),
        "selector_expression" => {
            let operand = node
                .child_by_field_name("operand")
                .map_or_else(|| "?".to_string(), |o| go_callee_name(o, src));
            let field = node.child_by_field_name("field").map_or("?", |f| text(f, src));
            format!("{operand}.{field}")
        }
        "call_expression" => {
            let function = node
                .child_by_field_name("function")
                .map_or_else(|| "?".to_string(), |f| go_callee_name(f, src));
            format!("{function}()")
        }
        "index_expression" => node
            .child_by_field_name("operand")
            .map_or_else(|| "?".to_string(), |o| go_callee_name(o, src)),
        "type_instantiation_expression" | "generic_type" => node
            .child_by_field_name("type")
            .map_or_else(|| "?".to_string(), |t| go_callee_name(t, src)),
        "qualified_type" => {
            let package = node.child_by_field_name("package").map_or("?", |p| text(p, src));
            format!("{package}.{}", node.child_by_field_name("name").map_or("?", |n| text(n, src)))
        }
        "parenthesized_expression" => first_named_child(node).map_or_else(|| "?".to_string(), |n| go_callee_name(n, src)),
        _ => "?".to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// HTML. Markup has no calls, raises or imports: the checks are elements and attributes.
// ---------------------------------------------------------------------------------------------

struct Html;

impl Adapter for Html {
    fn raised_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    fn call_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// An opening or self-closing tag, named in lowercase because HTML tag names are case-insensitive.
    fn element_name(&self, node: Node, src: &str) -> Option<String> {
        if !matches!(node.kind(), "start_tag" | "self_closing_tag") {
            return None;
        }
        let mut cursor = node.walk();
        let name = node.named_children(&mut cursor).find(|c| c.kind() == "tag_name")?;
        Some(text(name, src).to_ascii_lowercase())
    }

    /// An attribute is a property: `class="x"`, or a bare `disabled` with no value.
    fn object_property<'t>(&self, node: Node<'t>, src: &str) -> Option<Property<'t>> {
        if node.kind() != "attribute" {
            return None;
        }
        let mut cursor = node.walk();
        let mut children = node.named_children(&mut cursor);
        let name = children.find(|c| c.kind() == "attribute_name")?;
        let value = children.find(|c| matches!(c.kind(), "quoted_attribute_value" | "attribute_value"));
        Some(Property { key: text(name, src).to_ascii_lowercase(), value })
    }

    fn is_string_literal(&self, node: Node, _src: &str) -> bool {
        matches!(node.kind(), "quoted_attribute_value" | "attribute_value")
    }
}

// ---------------------------------------------------------------------------------------------
// Shell (bash and POSIX sh). No exceptions: the checks are commands and `source`d files.
// ---------------------------------------------------------------------------------------------

struct Shell;

impl Adapter for Shell {
    fn raised_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// A command by its name as written (`curl`, `./deploy.sh`, `/bin/rm`). A name built from an
    /// expansion (`$CMD`, `$(which x)`) is `?`.
    fn call_name(&self, node: Node, src: &str) -> Option<String> {
        if node.kind() != "command" {
            return None;
        }
        let name = node.child_by_field_name("name")?;
        Some(Self::word_value(first_named_child(name)?, src).unwrap_or_else(|| "?".to_string()))
    }

    /// The words after the command name; options such as `-f` count as arguments.
    fn call_arguments<'t>(&self, node: Node<'t>) -> Option<Vec<Node<'t>>> {
        if node.kind() != "command" {
            return None;
        }
        let mut cursor = node.walk();
        Some(node.children_by_field_name("argument", &mut cursor).collect())
    }

    fn string_value(&self, node: Node, src: &str) -> Option<String> {
        Self::word_value(node, src)
    }

    /// `source file` and `. file`, with a plain word as the file.
    fn imports(&self, node: Node, src: &str, file_path: &str, _project: &Project) -> Vec<ImportRef> {
        if !matches!(self.call_name(node, src).as_deref(), Some("source" | ".")) {
            return Vec::new();
        }
        let Some(first) = self.call_arguments(node).and_then(|args| args.first().copied()) else {
            return Vec::new();
        };
        let Some(specifier) = Self::word_value(first, src) else { return Vec::new() };
        let resolved = match relative_path(&specifier, file_path).filter(|_| specifier.starts_with('.')) {
            Some(path) => path,
            None => specifier.clone(),
        };
        vec![ImportRef { specifier, resolved }]
    }
}

impl Shell {
    /// The text of a bare word, a single-quoted string, or a double-quoted string without expansions.
    fn word_value(node: Node, src: &str) -> Option<String> {
        let raw = text(node, src);
        match node.kind() {
            "word" | "number" => Some(raw.to_string()),
            "raw_string" => raw.get(1..raw.len().checked_sub(1)?).map(str::to_string),
            "string" => {
                let mut cursor = node.walk();
                if node.named_children(&mut cursor).any(|c| c.kind() != "string_content") {
                    return None;
                }
                let inner = raw.get(1..raw.len().checked_sub(1)?)?;
                (!inner.contains(['\\', '$', '`'])).then(|| inner.to_string())
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// CSS. Functions (`rgb()`, `url()`), `@import`, and declaration names.
// ---------------------------------------------------------------------------------------------

struct Css;

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
