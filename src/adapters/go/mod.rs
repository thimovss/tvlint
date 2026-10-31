//! Go. No exceptions either: the checks are calls (`fmt.Println`, `db.Query`) and import paths,
//! which resolve to project directories through the module paths in `go.mod`.

mod gomod;

use tree_sitter::Node;

use super::{Adapter, ImportRef, first_named_child, text};
use crate::check::Project;

pub use gomod::GoModules;

pub(super) struct Go;

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
                let resolved = project.resolvers.go.resolve(&specifier).unwrap_or_else(|| specifier.clone());
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
