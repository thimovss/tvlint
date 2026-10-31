mod check;
mod config;
mod source;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};

use check::{ClassIndex, CompiledRule, Violation, check_file};
use config::{Config, Scope};

#[derive(Parser)]
#[command(name = "prlint", version, about = "Enforce your team's code rules deterministically")]
struct Cli {
    /// Path to the config file; the project root is its directory
    #[arg(short, long, global = true, default_value = "prlint.yaml")]
    config: PathBuf,

    /// Defaults to `check` when omitted
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Check the project against all rules (exit 1 if any violation is found)
    Check,
    /// Show why a rule exists, what it forbids and where it applies
    Explain {
        /// Rule id, as listed by `prlint list`
        rule: String,
    },
    /// List all configured rules
    List,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Command::Check) {
        Command::Check => check(&cli.config).map(|violations| u8::from(violations > 0)),
        Command::Explain { rule } => explain(&cli.config, &rule).map(|()| 0),
        Command::List => list(&cli.config).map(|()| 0),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("prlint: {err:#}");
            ExitCode::from(2)
        }
    }
}

/// Returns the number of violations found.
fn check(config_path: &Path) -> Result<usize> {
    let config = Config::load(config_path)?;
    let root = std::path::absolute(config_path)?
        .parent()
        .map(Path::to_path_buf)
        .context("config file has no parent directory")?;

    let rules = config
        .rules
        .iter()
        .map(|(id, rule)| Ok((id.as_str(), CompiledRule::new(id, rule)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
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


fn explain(config_path: &Path, id: &str) -> Result<()> {
    let config = Config::load(config_path)?;
    let Some(rule) = config.rules.get(id) else {
        let known: Vec<&str> = config.rules.keys().map(String::as_str).collect();
        bail!("unknown rule `{id}`; known rules: {}", known.join(", "));
    };
    println!("{id} ({:?})\n", rule.language);
    println!("Why:  {}", rule.rationale.trim());
    println!("Fix:  {}\n", rule.fix.trim());
    println!("Forbids:");
    if !rule.forbid_raise.is_empty() {
        println!("  raising {}", rule.forbid_raise.join(", "));
    }
    if !rule.forbid_call.is_empty() {
        println!("  calling {}", rule.forbid_call.join(", "));
    }
    if let Some(catch) = &rule.forbid_catch {
        println!("  catching {} or its subclasses", catch.subclass_of);
    }
    println!("\nApplies to:");
    let mut applied = false;
    for scope in config.scopes.iter().filter(|s| s.rules.iter().any(|r| r == id)) {
        applied = true;
        println!("  include: {}", scope.include.join(", "));
        if !scope.exclude.is_empty() {
            println!("  exclude: {}", scope.exclude.join(", "));
        }
    }
    if !applied {
        println!("  (no scope, this rule never runs)");
    }
    Ok(())
}

fn list(config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;
    let width = config.rules.keys().map(String::len).max().unwrap_or(0);
    for (id, rule) in &config.rules {
        let scopes = config.scopes.iter().filter(|s| s.rules.contains(id)).count();
        println!("{id:<width$}  {:?}  {scopes} scope(s)", rule.language);
    }
    Ok(())
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
