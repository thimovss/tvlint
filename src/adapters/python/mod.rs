//! Python. Imports resolve to project files through the source roots in `pyproject.toml`.

mod roots;

use tree_sitter::Node;

use super::{Adapter, ClassMembers, ImportRef, Member, last_segment, text};
use crate::check::Project;

pub use roots::PythonRoots;

pub(super) struct Python;

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

    fn class_family(&self) -> Option<&'static str> {
        Some("python")
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
                // `Base[T]` is still Base.
                let base = if base.kind() == "subscript" { base.child_by_field_name("value").unwrap_or(base) } else { base };
                if let Some(dotted) = dotted_name(base, src) {
                    bases.push(last_segment(&dotted).to_string());
                }
            }
        }
        Some((name, bases))
    }

    /// Methods are `def`s in the class body, fields are annotated or assigned names (`x: int`, `x = 1`).
    fn class_members<'t>(&self, node: Node<'t>, src: &str) -> Option<ClassMembers<'t>> {
        if node.kind() != "class_definition" {
            return None;
        }
        let mut members = ClassMembers { methods: Vec::new(), fields: Vec::new() };
        let Some(body) = node.child_by_field_name("body") else { return Some(members) };
        let mut cursor = body.walk();
        for statement in body.named_children(&mut cursor) {
            let statement = match statement.kind() {
                "decorated_definition" => statement.child_by_field_name("definition").unwrap_or(statement),
                _ => statement,
            };
            match statement.kind() {
                "function_definition" => members.methods.extend(statement.child_by_field_name("name").map(|n| text(n, src).to_string())),
                "expression_statement" => {
                    let Some(assignment) = statement.named_child(0).filter(|n| n.kind() == "assignment") else { continue };
                    let Some(left) = assignment.child_by_field_name("left").filter(|n| n.kind() == "identifier") else { continue };
                    members.fields.push(Member { name: text(left, src).to_string(), node: left });
                }
                _ => {}
            }
        }
        Some(members)
    }

    fn function_name(&self, node: Node, src: &str) -> Option<String> {
        if node.kind() != "function_definition" {
            return None;
        }
        Some(text(node.child_by_field_name("name")?, src).to_string())
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
        project.resolvers.py.roots_for(file_path).to_vec()
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
            let module = crate::adapters::join(base, &parts[..length].join("/"));
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
