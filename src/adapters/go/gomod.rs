//! The module paths declared by the project's `go.mod` files.
//!
//! A `go.mod` without a `module` line is an error: import paths could not be resolved.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::adapters::join;

/// The module path declared by each `go.mod`, with the directory it sits in.
#[derive(Debug, Default)]
pub struct GoModules {
    /// Module path -> project directory.
    modules: BTreeMap<String, String>,
}

impl GoModules {
    pub fn load(root: &Path, paths: &[String]) -> Result<GoModules> {
        let mut modules = BTreeMap::new();
        for path in paths.iter().filter(|p| *p == "go.mod" || p.ends_with("/go.mod")) {
            let text = std::fs::read_to_string(root.join(path)).with_context(|| format!("cannot read {path}"))?;
            let Some(module) = module_path(&text) else {
                bail!("{path} has no `module` line, so import paths cannot be resolved");
            };
            let dir = path.strip_suffix("go.mod").unwrap_or("").trim_end_matches('/');
            modules.insert(module, dir.to_string());
        }
        Ok(GoModules { modules })
    }

    #[cfg(test)]
    pub fn for_test(modules: &[(&str, &str)]) -> GoModules {
        GoModules { modules: modules.iter().map(|(m, d)| (m.to_string(), d.to_string())).collect() }
    }

    /// The project directory an import path leads to, if it is inside one of the project's modules.
    pub fn resolve(&self, import: &str) -> Option<String> {
        let (module, dir) = self
            .modules
            .iter()
            .filter(|(module, _)| import == module.as_str() || import.strip_prefix(module.as_str()).is_some_and(|r| r.starts_with('/')))
            .max_by_key(|(module, _)| module.len())?;
        Some(join(dir, import[module.len()..].trim_start_matches('/')))
    }
}

/// The path on the `module` line of a `go.mod`, which may be quoted and followed by a comment.
fn module_path(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.split("//").next().unwrap_or("").trim();
        let rest = line.strip_prefix("module")?;
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        let path = rest.trim().trim_matches(['"', '`']);
        (!path.is_empty()).then(|| path.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_module_lines_may_be_quoted_and_commented() {
        assert_eq!(module_path("// c\nmodule github.com/acme/app // x\n\ngo 1.22\n").as_deref(), Some("github.com/acme/app"));
        assert_eq!(module_path("module \"example.com/m\"\n").as_deref(), Some("example.com/m"));
        assert_eq!(module_path("modulex y\n"), None);
    }

    #[test]
    fn go_imports_resolve_to_the_nearest_module() {
        let go = GoModules {
            modules: BTreeMap::from([
                ("github.com/acme/app".to_string(), String::new()),
                ("github.com/acme/app/tools".to_string(), "tools".to_string()),
                ("example.com/other".to_string(), "other".to_string()),
            ]),
        };
        assert_eq!(go.resolve("github.com/acme/app/internal/db").as_deref(), Some("internal/db"));
        assert_eq!(go.resolve("github.com/acme/app").as_deref(), Some(""));
        assert_eq!(go.resolve("github.com/acme/app/tools/gen").as_deref(), Some("tools/gen"));
        assert_eq!(go.resolve("example.com/other/x").as_deref(), Some("other/x"));
        assert_eq!(go.resolve("github.com/acme/application"), None);
        assert_eq!(go.resolve("fmt"), None);
    }
}
