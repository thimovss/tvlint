use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};
use ignore::WalkBuilder;
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

fn language_of(path: &Path) -> Option<Language> {
    match path.extension()?.to_str()? {
        "py" => Some(Language::Python),
        _ => None,
    }
}

fn python_parser() -> Result<Parser> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| anyhow!("cannot load python grammar: {e}"))?;
    Ok(parser)
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
    let mut parser = python_parser()?;
    for file in files.iter_mut().filter(|f| staged.contains(&f.path)) {
        let blob = git(root, &["show", &format!(":./{}", file.path)])?;
        let Ok(text) = String::from_utf8(blob) else { continue };
        let Some(tree) = parser.parse(&text, None) else { continue };
        file.text = text;
        file.tree = tree;
    }
    Ok(())
}

/// Finds and parses every supported source file under `root`, honouring `.gitignore`.
pub fn load_all(root: &Path) -> Result<Vec<SourceFile>> {
    let mut parser = python_parser()?;

    let mut files = Vec::new();
    for entry in WalkBuilder::new(root).build() {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(language) = language_of(path) else { continue };
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let Some(tree) = parser.parse(&text, None) else { continue };
        let relative = path.strip_prefix(root).unwrap_or(path);
        files.push(SourceFile {
            path: relative.to_string_lossy().replace('\\', "/"),
            language,
            text,
            tree,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
