//! Import aliases read from `tsconfig.json` and `jsconfig.json`, so `paths` is written once.
//!
//! Only what alias resolution needs is read: `compilerOptions.baseUrl`, `compilerOptions.paths`,
//! `extends` (local files and packages in `node_modules`) and `references`. Nothing is executed.
//! A config that cannot be read is an error, never silently skipped.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

const MAX_DEPTH: usize = 20;

/// One `paths` entry: `"@/*": ["./src/*"]` becomes prefix `@/`, suffix `` and target `src/*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alias {
    prefix: String,
    suffix: String,
    wildcard: bool,
    /// Project paths, in the order the config lists them. `*` stands for what the pattern matched.
    targets: Vec<String>,
}

impl Alias {
    /// What the specifier's `*` stands for, if the specifier matches this alias.
    fn capture<'s>(&self, specifier: &'s str) -> Option<&'s str> {
        if !self.wildcard {
            return (specifier == self.prefix).then_some("");
        }
        let rest = specifier.strip_prefix(self.prefix.as_str())?;
        rest.strip_suffix(self.suffix.as_str())
    }

    /// The project paths the specifier may lead to, best candidate first.
    pub fn candidates(&self, specifier: &str) -> Option<Vec<String>> {
        let captured = self.capture(specifier)?;
        Some(self.targets.iter().map(|t| t.replacen('*', captured, 1)).collect())
    }

    fn rank(&self) -> usize {
        self.prefix.len()
    }
}

/// The aliases in effect for each directory that has a `tsconfig.json` or `jsconfig.json`.
#[derive(Debug, Default)]
pub struct TsConfigs {
    by_dir: BTreeMap<String, Vec<Alias>>,
}

impl TsConfigs {
    /// Reads the nearest-config aliases of every `tsconfig.json`/`jsconfig.json` among `paths`
    /// (project paths, relative to `root`).
    pub fn load(root: &Path, paths: &[String]) -> Result<TsConfigs> {
        let mut found: BTreeMap<String, PathBuf> = BTreeMap::new();
        for path in paths {
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            // tsconfig.json wins over jsconfig.json in the same directory.
            match name {
                "tsconfig.json" => {
                    found.insert(dir.to_string(), root.join(path));
                }
                "jsconfig.json" => {
                    found.entry(dir.to_string()).or_insert_with(|| root.join(path));
                }
                _ => {}
            }
        }
        let root = normalize(root);
        let mut by_dir = BTreeMap::new();
        for (dir, file) in found {
            let aliases = aliases_of(&file, &root, 0).with_context(|| format!("cannot read import aliases from {}", display(&file, &root)))?;
            // Kept even when empty: the nearest config decides, as in TypeScript.
            by_dir.insert(dir, aliases);
        }
        Ok(TsConfigs { by_dir })
    }

    /// The aliases of the nearest config at or above the directory of `file_path`.
    pub fn aliases_for(&self, file_path: &str) -> &[Alias] {
        let mut dir = file_path.rsplit_once('/').map_or("", |(dir, _)| dir);
        loop {
            if let Some(aliases) = self.by_dir.get(dir) {
                return aliases;
            }
            match dir.rsplit_once('/') {
                Some((parent, _)) => dir = parent,
                None if dir.is_empty() => return &[],
                None => dir = "",
            }
        }
    }

    /// The best alias for `specifier` (the longest prefix, as in TypeScript), with its candidates.
    pub fn resolve(&self, specifier: &str, file_path: &str) -> Option<Vec<String>> {
        self.aliases_for(file_path)
            .iter()
            .filter_map(|a| a.candidates(specifier).map(|c| (a.rank(), c)))
            .max_by_key(|(rank, _)| *rank)
            .map(|(_, candidates)| candidates)
    }
}

/// What one config contributes: the `paths` of the config itself, then those of its references.
fn aliases_of(file: &Path, root: &Path, depth: usize) -> Result<Vec<Alias>> {
    if depth > MAX_DEPTH {
        bail!("references nest more than {MAX_DEPTH} levels deep");
    }
    let mut seen = Vec::new();
    let options = merged_options(file, &mut seen)?;
    let mut aliases = options.aliases(root);
    for reference in options.references {
        let target = if reference.extension().is_some_and(|e| e == "json") { reference } else { reference.join("tsconfig.json") };
        let inner = aliases_of(&target, root, depth + 1)
            .with_context(|| format!("referenced from {}", display(file, root)))?;
        for alias in inner {
            if !aliases.iter().any(|a| a.prefix == alias.prefix && a.wildcard == alias.wildcard && a.suffix == alias.suffix) {
                aliases.push(alias);
            }
        }
    }
    Ok(aliases)
}

/// `paths` as written: pattern and its targets, in order.
type PathEntries = Vec<(String, Vec<String>)>;

/// `compilerOptions` after following `extends`: the child wins, and a child's `paths` replaces the parent's.
#[derive(Default)]
struct Options {
    base_url: Option<PathBuf>,
    paths: Option<(PathEntries, PathBuf)>,
    references: Vec<PathBuf>,
}

impl Options {
    fn aliases(&self, root: &Path) -> Vec<Alias> {
        let Some((entries, defined_in)) = &self.paths else { return Vec::new() };
        // Targets are relative to `baseUrl`, or to the config that defines `paths` when there is none.
        let base = self.base_url.as_ref().unwrap_or(defined_in);
        let mut aliases = Vec::new();
        for (pattern, targets) in entries {
            let targets: Vec<String> = targets
                .iter()
                .filter_map(|t| {
                    let path = normalize(&base.join(t));
                    let relative = path.strip_prefix(root).ok()?;
                    Some(relative.to_string_lossy().replace('\\', "/"))
                })
                .collect();
            if targets.is_empty() {
                continue; // outside the project: nothing in it to match
            }
            let (prefix, suffix, wildcard) = match pattern.split_once('*') {
                Some((prefix, suffix)) => (prefix.to_string(), suffix.to_string(), true),
                None => (pattern.clone(), String::new(), false),
            };
            aliases.push(Alias { prefix, suffix, wildcard, targets });
        }
        aliases
    }
}

fn merged_options(file: &Path, seen: &mut Vec<PathBuf>) -> Result<Options> {
    let file = normalize(file);
    if seen.contains(&file) {
        bail!("`extends` loops back to {}", file.display());
    }
    if seen.len() > MAX_DEPTH {
        bail!("`extends` nests more than {MAX_DEPTH} levels deep");
    }
    seen.push(file.clone());
    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let json = read_jsonc(&file)?;
    let mut options = Options::default();
    // `extends` is a path or, since TypeScript 5, a list; later entries win over earlier ones.
    let extends: Vec<&str> = match json.get("extends") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        Some(_) => bail!("`extends` in {} must be a string or a list of strings", file.display()),
    };
    for spec in extends {
        let base = resolve_extends(spec, &dir).with_context(|| format!("`extends: {spec}` in {}", file.display()))?;
        let inherited = merged_options(&base, seen)?;
        options.base_url = inherited.base_url.or(options.base_url);
        options.paths = inherited.paths.or(options.paths);
    }
    if let Some(compiler) = json.get("compilerOptions") {
        if let Some(base_url) = compiler.get("baseUrl").and_then(Value::as_str) {
            options.base_url = Some(normalize(&dir.join(base_url)));
        }
        match compiler.get("paths") {
            None | Some(Value::Null) => {}
            Some(Value::Object(map)) => {
                let mut entries = Vec::new();
                for (pattern, targets) in map {
                    let Value::Array(targets) = targets else {
                        bail!("`paths.{pattern}` in {} must be a list of paths", file.display());
                    };
                    entries.push((pattern.clone(), targets.iter().filter_map(Value::as_str).map(str::to_string).collect()));
                }
                options.paths = Some((entries, dir.clone()));
            }
            Some(_) => bail!("`paths` in {} must be a mapping", file.display()),
        }
    }
    if let Some(Value::Array(references)) = json.get("references") {
        options.references = references
            .iter()
            .filter_map(|r| r.get("path").and_then(Value::as_str))
            .map(|p| normalize(&dir.join(p)))
            .collect();
    }
    seen.pop();
    Ok(options)
}

/// The file `extends` names: a relative path, or a package found in a `node_modules` above `dir`.
fn resolve_extends(spec: &str, dir: &Path) -> Result<PathBuf> {
    let with_extension = |p: PathBuf| if p.extension().is_some_and(|e| e == "json") { p } else { p.with_extension_appended("json") };
    if spec.starts_with("./") || spec.starts_with("../") || Path::new(spec).is_absolute() {
        let path = dir.join(spec);
        return first_existing([path.clone(), with_extension(path.clone()), path.join("tsconfig.json")])
            .with_context(|| format!("{} does not exist", normalize(&path).display()));
    }
    for ancestor in dir.ancestors() {
        let package = ancestor.join("node_modules").join(spec);
        if let Some(found) = first_existing([package.clone(), with_extension(package.clone()), package.join("tsconfig.json")]) {
            return Ok(found);
        }
    }
    bail!("package `{spec}` is not in any node_modules above {}", dir.display())
}

fn first_existing(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|p| p.is_file())
}

trait AppendExtension {
    fn with_extension_appended(&self, extension: &str) -> PathBuf;
}

impl AppendExtension for PathBuf {
    /// `base` becomes `base.json`, and `base.v2` becomes `base.v2.json`.
    fn with_extension_appended(&self, extension: &str) -> PathBuf {
        let mut name = self.clone().into_os_string();
        name.push(".");
        name.push(extension);
        PathBuf::from(name)
    }
}

fn read_jsonc(file: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    serde_json::from_str(&strip_jsonc(&text)).with_context(|| format!("{} is not valid JSON", file.display()))
}

/// Removes `//` and `/* */` comments and trailing commas, leaving strings alone.
fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                out.push('"');
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        out.push(chars[i + 1]);
                        i += 1;
                    } else if chars[i] == '"' {
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    // Keep line breaks so error positions still point at the right line.
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                i += 2;
            }
            ',' => {
                // A comma directly before `}` or `]` (ignoring whitespace and comments) is dropped.
                let rest: String = chars[i + 1..].iter().collect();
                let next = strip_leading_comments(&rest);
                if !matches!(next.trim_start().chars().next(), Some('}' | ']')) {
                    out.push(',');
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn strip_leading_comments(mut rest: &str) -> &str {
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("//") {
            rest = after.split_once('\n').map_or("", |(_, tail)| tail);
        } else if let Some(after) = rest.strip_prefix("/*") {
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
        } else {
            return rest;
        }
    }
}

/// Removes `.` and resolves `..` without touching the file system.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn display(file: &Path, root: &Path) -> String {
    normalize(file).strip_prefix(root).map_or_else(|_| file.display().to_string(), |p| p.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> (PathBuf, Vec<String>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!("tvl-tsconfig-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        (root, files.iter().map(|(p, _)| p.to_string()).collect())
    }

    fn load(files: &[(&str, &str)]) -> Result<TsConfigs> {
        let (root, paths) = project(files);
        TsConfigs::load(&root, &paths)
    }

    #[test]
    fn comments_and_trailing_commas_are_allowed() {
        let ts = load(&[(
            "tsconfig.json",
            "{\n  // comment\n  \"compilerOptions\": {\n    /* block */ \"paths\": { \"@/*\": [\"./src/*\",], }, \"x\": \"// not a comment\",\n  },\n}\n",
        )])
        .unwrap();
        assert_eq!(ts.resolve("@/api/x", "src/a.ts"), Some(vec!["src/api/x".to_string()]));
    }

    #[test]
    fn paths_follow_base_url_and_longest_prefix_wins() {
        let ts = load(&[(
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":"src","paths":{"@/*":["*"],"@/ui/*":["components/ui/*"],"config":["lib/config"]}}}"#,
        )])
        .unwrap();
        assert_eq!(ts.resolve("@/ui/button", "a.ts"), Some(vec!["src/components/ui/button".to_string()]));
        assert_eq!(ts.resolve("@/api", "a.ts"), Some(vec!["src/api".to_string()]));
        assert_eq!(ts.resolve("config", "a.ts"), Some(vec!["src/lib/config".to_string()]));
        assert_eq!(ts.resolve("config/x", "a.ts"), None);
        assert_eq!(ts.resolve("react", "a.ts"), None);
    }

    #[test]
    fn extends_is_followed_and_the_childs_paths_replace_the_parents() {
        let ts = load(&[
            ("base/tsconfig.base.json", r#"{"compilerOptions":{"paths":{"@lib/*":["../libs/*"],"@old/*":["../old/*"]}}}"#),
            ("app/tsconfig.json", r#"{"extends":"../base/tsconfig.base","compilerOptions":{}}"#),
            ("web/tsconfig.json", r#"{"extends":"../base/tsconfig.base.json","compilerOptions":{"paths":{"@web/*":["./src/*"]}}}"#),
        ])
        .unwrap();
        // Without baseUrl, targets are relative to the config that defines `paths`.
        assert_eq!(ts.resolve("@lib/x", "app/src/a.ts"), Some(vec!["libs/x".to_string()]));
        assert_eq!(ts.resolve("@web/x", "web/src/a.ts"), Some(vec!["web/src/x".to_string()]));
        assert_eq!(ts.resolve("@lib/x", "web/src/a.ts"), None);
    }

    #[test]
    fn the_nearest_config_wins_and_jsconfig_is_read() {
        let ts = load(&[
            ("tsconfig.json", r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#),
            ("pkg/jsconfig.json", r#"{"compilerOptions":{"paths":{"@/*":["./lib/*"]}}}"#),
            ("pkg/plain/tsconfig.json", "{}"),
        ])
        .unwrap();
        assert_eq!(ts.resolve("@/x", "src/a.ts"), Some(vec!["src/x".to_string()]));
        assert_eq!(ts.resolve("@/x", "pkg/deep/a.js"), Some(vec!["pkg/lib/x".to_string()]));
        // A nearer config without `paths` does not inherit from further up, as in TypeScript.
        assert_eq!(ts.resolve("@/x", "pkg/plain/a.ts"), None);
    }

    #[test]
    fn references_contribute_their_paths() {
        let ts = load(&[
            ("tsconfig.json", r#"{"files":[],"references":[{"path":"./tsconfig.app.json"},{"path":"./node"}]}"#),
            ("tsconfig.app.json", r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#),
            ("node/tsconfig.json", r##"{"compilerOptions":{"paths":{"#n/*":["./n/*"]}}}"##),
        ])
        .unwrap();
        assert_eq!(ts.resolve("@/x", "src/a.ts"), Some(vec!["src/x".to_string()]));
        assert_eq!(ts.resolve("#n/x", "src/a.ts"), Some(vec!["node/n/x".to_string()]));
    }

    #[test]
    fn packages_in_node_modules_can_be_extended() {
        let ts = load(&[
            ("node_modules/@acme/tsconfig/base.json", r#"{"compilerOptions":{"baseUrl":".","paths":{"~/*":["src/*"]}}}"#),
            ("tsconfig.json", r#"{"extends":"@acme/tsconfig/base.json"}"#),
        ])
        .unwrap();
        // `baseUrl` is relative to the config that sets it, which here is the package.
        assert_eq!(ts.resolve("~/x", "src/a.ts"), Some(vec!["node_modules/@acme/tsconfig/src/x".to_string()]));
    }

    #[test]
    fn broken_configs_are_errors_naming_the_file() {
        let err = load(&[("tsconfig.json", "{ not json")]).unwrap_err();
        assert!(format!("{err:#}").contains("tsconfig.json"), "{err:#}");
        let err = load(&[("tsconfig.json", r#"{"extends":"./missing"}"#)]).unwrap_err();
        assert!(format!("{err:#}").contains("does not exist"), "{err:#}");
        let err = load(&[("a/tsconfig.json", r#"{"extends":"../b.json"}"#), ("b.json", r#"{"extends":"./a/tsconfig.json"}"#)]).unwrap_err();
        assert!(format!("{err:#}").contains("loops"), "{err:#}");
        let err = load(&[("tsconfig.json", r#"{"extends":"nonexistent-package"}"#)]).unwrap_err();
        assert!(format!("{err:#}").contains("node_modules"), "{err:#}");
    }

    #[test]
    fn a_project_without_configs_has_no_aliases() {
        let ts = load(&[("src/a.ts", "")]).unwrap();
        assert_eq!(ts.resolve("@/x", "src/a.ts"), None);
    }
}
