mod adapter;
mod check;
mod config;
mod scope;
mod source;
mod validate;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use check::{ClassIndex, CompiledRule, Project, Violation, check_file, check_filename};
use config::Config;
use scope::CompiledScope;

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
    Check {
        /// Only check files staged in git (as they are in the index)
        #[arg(long)]
        staged: bool,
    },
    /// Show why a rule exists, what it forbids and where it applies
    Explain {
        /// Rule id, as listed by `prlint list`
        rule: String,
    },
    /// List all configured rules
    List,
    /// Check the config file for mistakes and explain every problem (exit 1 if invalid)
    Validate,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Command::Check { staged: false }) {
        Command::Check { staged } => check(&cli.config, staged).map(|violations| u8::from(violations > 0)),
        Command::Explain { rule } => explain(&cli.config, &rule).map(|()| 0),
        Command::List => list(&cli.config).map(|()| 0),
        Command::Validate => validate::run(&cli.config).map(|valid| u8::from(!valid)),
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
fn check(config_path: &Path, staged: bool) -> Result<usize> {
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

    let mut files = source::load_all(&root)?;
    // Staged mode still parses the whole project: subclass lookups need every class.
    let staged_paths = if staged {
        let paths = source::staged_paths(&root)?;
        source::use_index_contents(&mut files, &paths, &root)?;
        Some(paths)
    } else {
        None
    };
    let index = ClassIndex::build(&files);
    let all_paths = source::list_all(&root)?;
    let known_paths: std::collections::HashSet<String> = all_paths.iter().cloned().collect();
    let ctx = Project { index: &index, aliases: &config.aliases, paths: &known_paths };
    // A file can be matched by several scopes naming the same rule; run each pair once.
    let mut jobs: BTreeSet<(usize, &str)> = BTreeSet::new();
    for (file_idx, file) in files.iter().enumerate() {
        if staged_paths.as_ref().is_some_and(|paths| !paths.contains(&file.path)) {
            continue;
        }
        for scope in scopes.iter().filter(|s| s.matches(&file.path)) {
            for id in scope.rules {
                let rule = rules[id.as_str()].rule;
                if rule.has_code_checks() && rule.applies_to(file.language) {
                    jobs.insert((file_idx, id.as_str()));
                }
            }
        }
    }

    let mut violations: Vec<Violation> = Vec::new();
    let mut checked: BTreeSet<&str> = BTreeSet::new();
    for (file_idx, id) in jobs {
        let file = &files[file_idx];
        checked.insert(file.path.as_str());
        if file.has_syntax_errors() {
            eprintln!("prlint: warning: {} has syntax errors, results may be incomplete", file.path);
        }
        check_file(file, &rules[id], &ctx, &mut violations);
    }

    // File-name rules look at every file, not only the ones we can parse.
    let mut name_jobs: BTreeSet<(&str, &str)> = BTreeSet::new();
    for path in &all_paths {
        if staged_paths.as_ref().is_some_and(|paths| !paths.contains(path)) {
            continue;
        }
        for scope in scopes.iter().filter(|s| s.matches(path)) {
            for id in scope.rules {
                let rule = rules[id.as_str()].rule;
                if rule.require_filename.is_empty() {
                    continue;
                }
                if !rule.applies_to_path(path) {
                    continue;
                }
                name_jobs.insert((path.as_str(), id.as_str()));
            }
        }
    }
    for (path, id) in name_jobs {
        checked.insert(path);
        violations.extend(check_filename(path, &rules[id]));
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
    println!("{id} ({})\n", rule.language_label());
    println!("Why:  {}", rule.rationale.trim());
    println!("Fix:  {}\n", rule.fix.trim());
    println!("Checks:");
    for line in rule.describe_checks() {
        println!("  {line}");
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
        let scope_count = config.scopes.iter().filter(|s| s.rules.contains(id)).count();
        println!("{id:<width$}  {}  {scope_count} scope(s)", rule.language_label());
    }
    Ok(())
}
