use anyhow::{Context, Result};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use crate::config::{CapturePattern, Scope};

pub struct CompiledScope<'a> {
    pub rules: &'a [String],
    include: GlobSet,
    exclude: GlobSet,
    /// The same `include` patterns, able to say what their wildcard segments matched.
    captures: Vec<CapturePattern>,
}

impl<'a> CompiledScope<'a> {
    pub fn new(scope: &'a Scope) -> Result<Self> {
        Ok(CompiledScope {
            rules: &scope.rules,
            include: path_globs(&scope.include)?,
            exclude: path_globs(&scope.exclude)?,
            captures: scope.include.iter().map(|pattern| CapturePattern::new(pattern)).collect::<Result<_>>()?,
        })
    }

    /// What the wildcard segments of the first `include` pattern that matches `path` matched, in order.
    pub fn captures(&self, path: &str) -> Option<Vec<String>> {
        if !self.matches(path) {
            return None;
        }
        self.captures.iter().find_map(|pattern| pattern.captures(path))
    }

    pub fn matches(&self, path: &str) -> bool {
        self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

/// Path globs where `*` stays within one directory and `**` crosses directories.
pub fn path_globs(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob: Glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("invalid glob `{pattern}`"))?;
        builder.add(glob);
    }
    Ok(builder.build()?)
}
