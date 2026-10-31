//! The directories Python imports start from, as declared in `pyproject.toml`.

use std::collections::BTreeMap;

use crate::adapters::join;
use crate::data::Doc;

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
    use crate::source::{Parsers, SourceFile};

    fn roots(pyprojects: &[(&str, &str)], other_paths: &[&str]) -> PythonRoots {
        let mut parsers = Parsers::new().unwrap();
        let mut docs = BTreeMap::new();
        let mut paths: Vec<String> = other_paths.iter().map(|p| p.to_string()).collect();
        for (path, text) in pyprojects {
            let (language, tree) = parsers.parse(path, text).unwrap();
            let file = SourceFile { path: path.to_string(), language, text: text.to_string(), tree };
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
