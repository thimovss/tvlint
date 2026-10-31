//! The files, directories and globs given to `tvl check`, as a filter on project paths.
//!
//! Arguments are relative to the directory the user runs tvl from, which is not always the
//! project root (the directory of the config file), so they are turned into root-relative paths.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use globset::{GlobBuilder, GlobMatcher};

pub struct Selection {
    args: Vec<Arg>,
}

struct Arg {
    /// As the user typed it, for messages.
    label: String,
    kind: Kind,
}

enum Kind {
    File(String),
    /// A directory, as a root-relative path; empty for the root itself.
    Dir(String),
    Glob(GlobMatcher),
}

impl Selection {
    /// `root` is the project root and `cwd` the directory the arguments are relative to.
    pub fn new(args: &[String], root: &Path, cwd: &Path) -> Result<Selection> {
        let root = normalize(root);
        let mut parsed = Vec::new();
        for arg in args {
            let absolute = normalize(&cwd.join(arg));
            let Ok(relative) = absolute.strip_prefix(&root) else {
                bail!("`{arg}` is outside the project root {}", root.display());
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            // Something that exists wins over a glob reading of its name: `[id].tsx` is a real file.
            let kind = if absolute.is_dir() {
                Kind::Dir(relative)
            } else if absolute.is_file() {
                Kind::File(relative)
            } else if arg.contains(['*', '?', '[', '{']) {
                let glob = GlobBuilder::new(&relative)
                    .literal_separator(true)
                    .build()
                    .with_context(|| format!("invalid glob `{arg}`"))?;
                Kind::Glob(glob.compile_matcher())
            } else {
                bail!("`{arg}` does not exist");
            };
            parsed.push(Arg { label: arg.clone(), kind });
        }
        Ok(Selection { args: parsed })
    }

    pub fn matches(&self, path: &str) -> bool {
        self.args.iter().any(|arg| arg.matches(path))
    }

    /// The arguments, as typed, that match none of `paths`.
    pub fn unmatched<'a>(&'a self, paths: &[String]) -> Vec<&'a str> {
        self.args
            .iter()
            .filter(|arg| !paths.iter().any(|p| arg.matches(p)))
            .map(|arg| arg.label.as_str())
            .collect()
    }
}

impl Arg {
    fn matches(&self, path: &str) -> bool {
        match &self.kind {
            Kind::File(file) => path == file,
            Kind::Dir(dir) => dir.is_empty() || path.strip_prefix(dir.as_str()).is_some_and(|rest| rest.starts_with('/')),
            Kind::Glob(glob) => glob.is_match(path),
        }
    }
}

/// Resolves `.` and `..` without touching the file system, so paths that do not exist (globs) work.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A project on disk: web/src/a.ts, web/src/lib/b.ts, web/src/[id].tsx, api/m.py.
    fn project() -> PathBuf {
        let root = std::env::temp_dir().join(format!("tvl-select-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = fs::remove_dir_all(&root);
        for file in ["web/src/a.ts", "web/src/lib/b.ts", "web/src/[id].tsx", "api/m.py"] {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        root
    }

    fn select(root: &Path, cwd: &str, args: &[&str]) -> Result<Selection> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        Selection::new(&args, root, &root.join(cwd))
    }

    fn matching(s: &Selection) -> Vec<&'static str> {
        ["web/src/a.ts", "web/src/lib/b.ts", "web/src/[id].tsx", "api/m.py", "web/src/alpha/c.ts"]
            .into_iter()
            .filter(|p| s.matches(p))
            .collect()
    }

    #[test]
    fn a_file_matches_only_itself() {
        let root = project();
        let s = select(&root, "", &["web/src/a.ts"]).unwrap();
        assert_eq!(matching(&s), ["web/src/a.ts"]);
    }

    #[test]
    fn a_directory_matches_everything_below_it_but_not_a_name_prefix() {
        let root = project();
        let s = select(&root, "", &["web/src/lib"]).unwrap();
        assert_eq!(matching(&s), ["web/src/lib/b.ts"]);
        let s = select(&root, "", &["web/src/a"]);
        assert!(s.is_err(), "web/src/a is neither a file nor a directory");
        assert_eq!(matching(&select(&root, "", &["web"]).unwrap()), ["web/src/a.ts", "web/src/lib/b.ts", "web/src/[id].tsx", "web/src/alpha/c.ts"]);
    }

    #[test]
    fn globs_are_relative_to_the_current_directory() {
        let root = project();
        let from_root = select(&root, "", &["web/src/*.ts"]).unwrap();
        assert_eq!(matching(&from_root), ["web/src/a.ts"]);
        let from_web = select(&root, "web", &["src/**/*.ts"]).unwrap();
        assert_eq!(matching(&from_web), ["web/src/a.ts", "web/src/lib/b.ts", "web/src/alpha/c.ts"]);
        let sideways = select(&root, "web/src", &["../../api/*.py"]).unwrap();
        assert_eq!(matching(&sideways), ["api/m.py"]);
    }

    #[test]
    fn dot_means_the_current_directory() {
        let root = project();
        assert_eq!(matching(&select(&root, "api", &["."]).unwrap()), ["api/m.py"]);
        assert_eq!(matching(&select(&root, "", &["."]).unwrap()).len(), 5);
    }

    #[test]
    fn a_real_file_with_glob_characters_in_its_name_is_a_file() {
        let root = project();
        let s = select(&root, "", &["web/src/[id].tsx"]).unwrap();
        assert_eq!(matching(&s), ["web/src/[id].tsx"]);
    }

    #[test]
    fn several_arguments_add_up() {
        let root = project();
        let s = select(&root, "", &["api", "web/src/a.ts"]).unwrap();
        assert_eq!(matching(&s), ["web/src/a.ts", "api/m.py"]);
    }

    #[test]
    fn mistakes_are_errors() {
        let root = project();
        let outside = select(&root, "", &["../elsewhere/x.ts"]).err().unwrap().to_string();
        assert!(outside.contains("outside the project root"), "{outside}");
        let missing = select(&root, "", &["web/src/nope.ts"]).err().unwrap().to_string();
        assert!(missing.contains("does not exist"), "{missing}");
        let bad = select(&root, "", &["web/[*.ts"]).err().unwrap().to_string();
        assert!(bad.contains("invalid glob"), "{bad}");
    }

    #[test]
    fn unmatched_names_the_arguments_that_select_nothing() {
        let root = project();
        let s = select(&root, "", &["api", "web/src/*.js"]).unwrap();
        let paths = vec!["api/m.py".to_string(), "web/src/a.ts".to_string()];
        assert_eq!(s.unmatched(&paths), ["web/src/*.js"]);
    }
}
