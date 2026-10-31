//! The engine: runs compiled rules against parsed files and reports violations.

mod classes;
mod code;
mod compile;
mod data_checks;
mod structure;

pub use classes::ClassIndex;
#[cfg(test)]
pub use code::check_file;
pub use code::{check_file_in_scope, unused_registry_entries, visit};
pub use compile::{CompiledLiteral, CompiledRule, Literal, normalize_file};

use std::collections::{BTreeMap, HashSet};

use crate::data::Doc;

/// What a check needs to know about the rest of the project.
#[derive(Clone, Copy)]
pub struct Project<'a> {
    pub index: &'a ClassIndex,
    /// Import prefix -> project path, e.g. `@/` -> `src/`.
    pub aliases: &'a BTreeMap<String, String>,
    /// What `tsconfig.json`, `go.mod` and `pyproject.toml` say about where imports lead (owned by the adapters). The `aliases` of the rule's config win over tsconfig.
    pub resolvers: &'a crate::adapters::Resolvers,
    /// Every file in the project, to find which file an import points at.
    pub paths: &'a HashSet<String>,
    /// Every JSON, YAML and TOML file, parsed.
    pub docs: &'a BTreeMap<String, Doc>,
}

pub struct Violation {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub rule_id: String,
    pub message: String,
}

/// A file whose name matches none of the rule's `require_filename` globs.
pub fn check_filename(path: &str, rule: &CompiledRule) -> Option<Violation> {
    if rule.rule.require_filename.is_empty() {
        return None;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    if rule.filename.is_match(name) {
        return None;
    }
    Some(Violation {
        path: path.to_string(),
        line: 1,
        column: 1,
        rule_id: rule.id.to_string(),
        message: format!(
            "file name `{name}` is not allowed here; allowed names: {}",
            rule.rule.require_filename.join(", ")
        ),
    })
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod structure_tests;
