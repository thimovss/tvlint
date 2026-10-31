use std::path::Path;

use anyhow::{Result, anyhow};
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

/// Finds and parses every supported source file under `root`, honouring `.gitignore`.
pub fn load_all(root: &Path) -> Result<Vec<SourceFile>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| anyhow!("cannot load python grammar: {e}"))?;

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
