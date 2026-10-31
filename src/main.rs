mod check;
mod config;
mod source;

use std::collections::BTreeSet;
use std::process::ExitCode;

use anyhow::{Context, Result};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use check::{ClassIndex, CompiledRule, Violation, check_file};
use config::{Config, Scope};

const CONFIG_FILE: &str = "prlint.yaml";

fn main() -> ExitCode {
    match run() {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(err) => {
            eprintln!("prlint: {err:#}");
            ExitCode::from(2)
        }
    }
}

/// Returns the number of violations found.
fn run() -> Result<usize> {
    let root = std::env::current_dir().context("cannot determine current directory")?;
    let config = Config::load(&root.join(CONFIG_FILE))?;

    let rules = config
        .rules
        .iter()
        .map(|(id, rule)| Ok((id.as_str(), CompiledRule::new(id, rule)?)))
        .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
    let scopes = config.scopes.iter().map(CompiledScope::new).collect::<Result<Vec<_>>>()?;

    let files = source::load_all(&root)?;
    let index = ClassIndex::build(&files);

    // A file can be matched by several scopes naming the same rule; run each pair once.
    let mut jobs: BTreeSet<(usize, &str)> = BTreeSet::new();
    for (file_idx, file) in files.iter().enumerate() {
        for scope in scopes.iter().filter(|s| s.matches(&file.path)) {
            for id in scope.rules {
                if rules[id.as_str()].rule.language == file.language {
                    jobs.insert((file_idx, id.as_str()));
                }
            }
        }
    }

    let mut violations: Vec<Violation> = Vec::new();
    let mut checked = BTreeSet::new();
    for (file_idx, id) in jobs {
        let file = &files[file_idx];
        checked.insert(file_idx);
        if file.has_syntax_errors() {
            eprintln!("prlint: warning: {} has syntax errors, results may be incomplete", file.path);
        }
        check_file(file, &rules[id], &index, &mut violations);
    }

    violations.sort_by(|a, b| (&a.path, a.line, a.column, &a.rule_id).cmp(&(&b.path, b.line, b.column, &b.rule_id)));
    for v in &violations {
        let rule = rules[v.rule_id.as_str()].rule;
        println!("{}:{}:{}: error[{}] {}", v.path, v.line, v.column, v.rule_id, v.message);
        println!("    why: {}", rule.rationale.trim());
        println!("    fix: {}", rule.fix.trim());
    }
    if violations.is_empty() {
        println!("prlint: no violations ({} files checked)", checked.len());
    } else {
        println!("prlint: {} violation(s) in {} file(s)", violations.len(), {
            violations.iter().map(|v| &v.path).collect::<BTreeSet<_>>().len()
        });
    }
    Ok(violations.len())
}

struct CompiledScope<'a> {
    rules: &'a [String],
    include: GlobSet,
    exclude: GlobSet,
}

impl<'a> CompiledScope<'a> {
    fn new(scope: &'a Scope) -> Result<Self> {
        Ok(CompiledScope {
            rules: &scope.rules,
            include: path_globs(&scope.include)?,
            exclude: path_globs(&scope.exclude)?,
        })
    }

    fn matches(&self, path: &str) -> bool {
        self.include.is_match(path) && !self.exclude.is_match(path)
    }
}

/// Path globs where `*` stays within one directory and `**` crosses directories.
fn path_globs(patterns: &[String]) -> Result<GlobSet> {
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
