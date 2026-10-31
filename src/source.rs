use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use ignore::WalkBuilder;
use rayon::prelude::*;
use tree_sitter::{Parser, Tree};

use crate::config::Language;

pub struct SourceFile {
    /// Path relative to the project root, with `/` separators.
    pub path: String,
    pub language: Language,
    pub text: String,
    pub tree: Tree,
}

impl SourceFile {
    pub fn has_syntax_errors(&self) -> bool {
        self.tree.root_node().has_error()
    }
}

/// One tree-sitter parser per grammar. TSX needs its own grammar, so `.tsx` is told apart by extension.
pub struct Parsers {
    python: Parser,
    javascript: Parser,
    typescript: Parser,
    tsx: Parser,
    rust: Parser,
    go: Parser,
    html: Parser,
    json: Parser,
    yaml: Parser,
    toml: Parser,
}

impl Parsers {
    pub fn new() -> Result<Parsers> {
        fn parser(name: &str, language: tree_sitter::Language) -> Result<Parser> {
            let mut parser = Parser::new();
            parser.set_language(&language).map_err(|e| anyhow!("cannot load {name} grammar: {e}"))?;
            Ok(parser)
        }
        Ok(Parsers {
            python: parser("python", tree_sitter_python::LANGUAGE.into())?,
            javascript: parser("javascript", tree_sitter_javascript::LANGUAGE.into())?,
            typescript: parser("typescript", tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())?,
            tsx: parser("tsx", tree_sitter_typescript::LANGUAGE_TSX.into())?,
            rust: parser("rust", tree_sitter_rust::LANGUAGE.into())?,
            go: parser("go", tree_sitter_go::LANGUAGE.into())?,
            html: parser("html", tree_sitter_html::LANGUAGE.into())?,
            json: parser("json", tree_sitter_json::LANGUAGE.into())?,
            yaml: parser("yaml", tree_sitter_yaml::LANGUAGE.into())?,
            toml: parser("toml", tree_sitter_toml_ng::LANGUAGE.into())?,
        })
    }

    /// Parses `text` with the grammar for `path`'s extension. `None` for unsupported files.
    pub fn parse(&mut self, path: &str, text: &str) -> Option<(Language, Tree)> {
        let path = Path::new(path);
        let language = Language::from_path(path)?;
        let parser = match language {
            Language::Python => &mut self.python,
            Language::JavaScript => &mut self.javascript,
            Language::TypeScript if path.extension().is_some_and(|e| e == "tsx") => &mut self.tsx,
            Language::TypeScript => &mut self.typescript,
            Language::Rust => &mut self.rust,
            Language::Go => &mut self.go,
            Language::Html => &mut self.html,
            Language::Json => &mut self.json,
            Language::Yaml => &mut self.yaml,
            Language::Toml => &mut self.toml,
        };
        parser.parse(text, None).map(|tree| (language, tree))
    }
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .context("cannot run git")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("git {} failed: {}", args.join(" "), stderr.lines().next().unwrap_or("").trim());
    }
    Ok(output.stdout)
}

/// Paths (relative to `root`) of files that are added, copied, modified or renamed in the index.
pub fn staged_paths(root: &Path) -> Result<BTreeSet<String>> {
    // Outside a repository `git diff` silently falls back to a different mode, so check first.
    if git(root, &["rev-parse", "--is-inside-work-tree"]).is_err() {
        bail!("--staged needs a git repository, but {} is not inside one", root.display());
    }
    let out = git(root, &["diff", "--cached", "--relative", "--name-only", "--diff-filter=ACMR", "-z"])?;
    Ok(out
        .split(|&b| b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

/// Replaces the working-tree contents of staged files with what is in the index,
/// so unstaged edits to a staged file are not checked.
pub fn use_index_contents(files: &mut [SourceFile], staged: &BTreeSet<String>, root: &Path) -> Result<()> {
    let mut parsers = Parsers::new()?;
    for file in files.iter_mut().filter(|f| staged.contains(&f.path)) {
        let blob = git(root, &["show", &format!(":./{}", file.path)])?;
        let Ok(text) = String::from_utf8(blob) else { continue };
        let Some((_, tree)) = parsers.parse(&file.path, &text) else { continue };
        file.text = text;
        file.tree = tree;
    }
    Ok(())
}

/// Walks the project honouring `.gitignore`. Hidden files count (`.github/workflows` holds rules' targets), `.git` does not.
fn walker(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder.hidden(false).filter_entry(|entry| entry.file_name() != ".git");
    builder
}

/// Every file under `root` (any type), relative to it, honouring `.gitignore`.
pub fn list_all(root: &Path) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for entry in walker(root).build() {
        let entry = entry?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        paths.push(relative.to_string_lossy().replace('\\', "/"));
    }
    paths.sort();
    Ok(paths)
}

/// Config files below the project root (relative to it), parents before children. `own` is the root config itself.
pub fn find_configs(root: &Path, own: &Path) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for entry in walker(root).build() {
        let entry = entry?;
        if entry.file_type().is_some_and(|t| t.is_file()) && entry.file_name() == crate::config::FILE_NAME && entry.path() != own {
            let relative = entry.path().strip_prefix(root).unwrap_or(entry.path()).to_string_lossy().replace('\\', "/");
            if relative.contains('/') {
                found.push(relative);
            }
        }
    }
    found.sort_by(|a, b| (a.matches('/').count(), a).cmp(&(b.matches('/').count(), b)));
    Ok(found)
}

/// The config at `path` with the nested configs below its directory.
pub fn load_config(path: &Path) -> Result<crate::config::Config> {
    let own = std::path::absolute(path)?;
    let root = own.parent().context("config file has no parent directory")?;
    crate::config::Config::load_tree(path, &find_configs(root, &own)?)
}

/// Finds and parses every supported source file under `root`, honouring `.gitignore`.
/// Files are parsed in parallel, each worker thread with parsers of its own.
pub fn load_all(root: &Path) -> Result<Vec<SourceFile>> {
    let mut paths = Vec::new();
    for entry in walker(root).build() {
        let entry = entry?;
        if entry.file_type().is_some_and(|t| t.is_file()) && Language::from_path(entry.path()).is_some() {
            paths.push(entry.into_path());
        }
    }
    let mut files: Vec<SourceFile> = paths
        .par_iter()
        .map_init(Parsers::new, |parsers, path| {
            let parsers = parsers.as_mut().ok()?;
            let text = std::fs::read_to_string(path).ok()?;
            let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
            let (language, tree) = parsers.parse(&relative, &text)?;
            Some(SourceFile { path: relative, language, text, tree })
        })
        .flatten()
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
