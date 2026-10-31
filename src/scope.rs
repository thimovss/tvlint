use anyhow::{Context, Result};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use crate::config::Scope;

pub struct CompiledScope<'a> {
    pub rules: &'a [String],
    include: GlobSet,
    exclude: GlobSet,
}

impl<'a> CompiledScope<'a> {
    pub fn new(scope: &'a Scope) -> Result<Self> {
        Ok(CompiledScope {
            rules: &scope.rules,
            include: path_globs(&scope.include)?,
            exclude: path_globs(&scope.exclude)?,
        })
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
