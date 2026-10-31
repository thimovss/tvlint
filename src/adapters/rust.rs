//! Rust. No exceptions: the checks are calls (functions, methods and macros) and `use` paths.

use tree_sitter::Node;

use super::{Adapter, ImportRef, first_named_child, text};
use crate::check::Project;

pub(super) struct Rust;

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
