//! Where imports lead, from the files each ecosystem already keeps for it:
//! `tsconfig.json` (JavaScript, TypeScript), `go.mod` (Go) and `pyproject.toml` (Python).
//!
//! Nothing is executed. A `go.mod` that cannot be read stops the run with exit code 2.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::data::Doc;
use crate::tsconfig::TsConfigs;

/// Everything read from project files to resolve imports.
#[derive(Debug, Default)]
pub struct Modules {
    pub ts: TsConfigs,
    pub go: GoModules,
    pub py: PythonRoots,
}

/// `a/b` joined with `c`; either side may be empty.
pub fn join(dir: &str, rest: &str) -> String {
    match (dir.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (_, true) => dir.to_string(),
        _ => format!("{dir}/{rest}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Go
// ---------------------------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------------------------

/// The directories absolute imports start from, per `pyproject.toml`.
#[derive(Debug, Default)]
pub struct PythonRoots {
    /// Project directory of a `pyproject.toml` -> source roots, best first.
    by_dir: BTreeMap<String, Vec<String>>,
    /// Used when no `pyproject.toml` is above a file.
    fallback: Vec<String>,
}

impl PythonRoots {
    /// `docs` holds the parsed `pyproject.toml` files; `paths` is every file in the project.
    pub fn load(docs: &BTreeMap<String, Doc>, paths: &[String]) -> PythonRoots {
        let has_dir = |dir: &str| paths.iter().any(|p| p.starts_with(&format!("{}/", dir)));
        let implicit = |base: &str| {
            let mut roots = vec![base.to_string()];
            let src = join(base, "src");
            if has_dir(&src) {
                roots.push(src);
            }
            roots
        };
        let mut by_dir = BTreeMap::new();
        for (path, doc) in docs.iter().filter(|(p, _)| *p == "pyproject.toml" || p.ends_with("/pyproject.toml")) {
            let dir = path.strip_suffix("pyproject.toml").unwrap_or("").trim_end_matches('/').to_string();
            let mut roots = Vec::new();
            doc.walk(&mut |keys, _, node| {
                let Some(value) = node.scalar().map(|s| s.text.as_str()) else { return };
                let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
                let declared = match keys.as_slice() {
                    ["tool", "setuptools", "packages", "find", "where", _] => Some(value.to_string()),
                    ["tool", "setuptools", "package-dir", ""] => Some(value.to_string()),
                    ["tool", "poetry", "packages", _, "from"] => Some(value.to_string()),
                    ["tool", "pytest", "ini_options", "pythonpath", _] => Some(value.to_string()),
                    // `packages = ["src/app"]`: imports start in the folder that holds `app`.
                    ["tool", "hatch", "build", "targets", "wheel", "packages", _] => {
                        value.rsplit_once('/').map(|(parent, _)| parent.to_string())
                    }
                    _ => None,
                };
                if let Some(declared) = declared {
                    let root = join(&dir, declared.trim_start_matches("./").trim_end_matches('/'));
                    let root = if root == "." { String::new() } else { root };
                    if !roots.contains(&root) {
                        roots.push(root);
                    }
                }
            });
            for root in implicit(&dir) {
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
            by_dir.insert(dir, roots);
        }
        PythonRoots { by_dir, fallback: implicit("") }
    }

    /// The roots of the nearest `pyproject.toml` above `file_path`.
    pub fn roots_for(&self, file_path: &str) -> &[String] {
        let mut dir = file_path.rsplit_once('/').map_or("", |(dir, _)| dir);
        loop {
            if let Some(roots) = self.by_dir.get(dir) {
                return roots;
            }
            match dir.rsplit_once('/') {
                Some((parent, _)) => dir = parent,
                None if dir.is_empty() => return &self.fallback,
                None => dir = "",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Parsers;

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

    fn roots(pyprojects: &[(&str, &str)], other_paths: &[&str]) -> PythonRoots {
        let mut parsers = Parsers::new().unwrap();
        let mut docs = BTreeMap::new();
        let mut paths: Vec<String> = other_paths.iter().map(|p| p.to_string()).collect();
        for (path, text) in pyprojects {
            let (language, tree) = parsers.parse(path, text).unwrap();
            let file = crate::source::SourceFile { path: path.to_string(), language, text: text.to_string(), tree };
            docs.insert(path.to_string(), Doc::build(&file));
            paths.push(path.to_string());
        }
        PythonRoots::load(&docs, &paths)
    }

    #[test]
    fn python_roots_come_from_the_tools_that_declare_them() {
        let r = roots(
            &[
                ("pyproject.toml", "[tool.setuptools.packages.find]\nwhere = [\"lib\"]\n[tool.pytest.ini_options]\npythonpath = [\".\", \"tests/helpers\"]\n"),
                ("svc/pyproject.toml", "[tool.poetry]\npackages = [{ include = \"svc\", from = \"python\" }]\n"),
                ("h/pyproject.toml", "[tool.hatch.build.targets.wheel]\npackages = [\"src/h\"]\n"),
            ],
            &["src/app/a.py", "h/src/h/a.py"],
        );
        assert_eq!(r.roots_for("x/a.py"), ["lib", "", "tests/helpers", "src"]);
        assert_eq!(r.roots_for("svc/app/a.py"), ["svc/python", "svc"]);
        assert_eq!(r.roots_for("h/src/h/a.py"), ["h/src", "h"]);
    }

    #[test]
    fn without_a_pyproject_the_project_root_and_src_are_roots() {
        let r = roots(&[], &["src/app/a.py"]);
        assert_eq!(r.roots_for("src/app/a.py"), ["", "src"]);
        let r = roots(&[], &["app/a.py"]);
        assert_eq!(r.roots_for("app/a.py"), [""]);
    }
}
