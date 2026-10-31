//! Shell (bash and POSIX sh). No exceptions: the checks are commands and `source`d files.

use tree_sitter::Node;

use super::{Adapter, ImportRef, first_named_child, relative_path, text};
use crate::check::Project;

pub(super) struct Shell;

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
