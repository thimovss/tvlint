mod adapter;
mod check;
mod config;
mod data;
mod init;
mod scope;
mod select;
mod source;
mod suppress;
mod validate;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use rayon::prelude::*;

use check::{ClassIndex, CompiledRule, Project, Violation, check_file, check_filename, normalize_file, unused_registry_entries};
use config::Config;
use scope::CompiledScope;
use suppress::Suppressions;

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
        /// Files, directories or glob patterns to check, relative to the current directory.
        /// Quote globs so prlint expands them (`"src/**/*.ts"`). Default: the whole project
        #[arg(value_name = "PATH")]
        paths: Vec<String>,
        /// Only check files staged in git (as they are in the index). Combined with PATH, a
        /// file must be both staged and selected
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
    /// Create a starter prlint.yaml, with rules chosen for the languages found in the project
    Init {
        /// Overwrite the config file if it already exists
        #[arg(long)]
        force: bool,
    },
    /// Print the JSON Schema of the config file, for editors and AI agents that write rules
    Schema,
}

pub const SCHEMA: &str = include_str!("../prlint.schema.json");

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command.unwrap_or(Command::Check { paths: Vec::new(), staged: false }) {
        Command::Check { paths, staged } => check(&cli.config, &paths, staged).map(|violations| u8::from(violations > 0)),
        Command::Explain { rule } => explain(&cli.config, &rule).map(|()| 0),
        Command::List => list(&cli.config).map(|()| 0),
        Command::Validate => validate::run(&cli.config).map(|valid| u8::from(!valid)),
        Command::Init { force } => init::run(&cli.config, force).map(|()| 0),
        Command::Schema => {
            print!("{SCHEMA}");
            Ok(0)
        }
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
fn check(config_path: &Path, paths: &[String], staged: bool) -> Result<usize> {
    let config = Config::load(config_path)?;
    let root = std::path::absolute(config_path)?
        .parent()
        .map(Path::to_path_buf)
        .context("config file has no parent directory")?;

    let selection = if paths.is_empty() {
        None
    } else {
        let cwd = std::env::current_dir().context("cannot determine current directory")?;
        Some(select::Selection::new(paths, &root, &cwd)?)
    };

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
    // The class hierarchy still includes files that opt out, so other files' results do not change.
    let index = ClassIndex::build(&files);
    // A file is checked when it is staged (if --staged) and selected by the PATH arguments (if any).
    let selected = |path: &str| {
        staged_paths.as_ref().is_none_or(|staged| staged.contains(path))
            && selection.as_ref().is_none_or(|selection| selection.matches(path))
    };
    let suppressions: Vec<Suppressions> = files.iter().map(Suppressions::scan).collect();
    let ignored: BTreeSet<&str> = files
        .iter()
        .zip(&suppressions)
        .filter(|(_, s)| s.ignore_file)
        .map(|(f, _)| f.path.as_str())
        .collect();
    let all_paths = source::list_all(&root)?;
    for unmatched in selection.iter().flat_map(|s| s.unmatched(&all_paths)) {
        eprintln!("prlint: warning: `{unmatched}` matches no file (files ignored by .gitignore are skipped)");
    }
    let known_paths: std::collections::HashSet<String> = all_paths.iter().cloned().collect();
    let docs: BTreeMap<String, data::Doc> =
        files.iter().filter(|f| f.language.is_data()).map(|f| (f.path.clone(), data::Doc::build(f))).collect();
    for (id, rule) in &config.rules {
        let reads = rule.same_keys_as.iter().chain(rule.literal_argument.iter().map(|a| &a.registry.file));
        if let Some(file) = reads.map(|f| normalize_file(f)).find(|f| !docs.contains_key(f)) {
            bail!("rule `{id}` reads {file}, which is not a JSON, YAML or TOML file in the project (paths are relative to the config; ignored files are skipped)");
        }
    }
    let ctx = Project { index: &index, aliases: &config.aliases, paths: &known_paths, docs: &docs };
    // A file can be matched by several scopes naming the same rule; run each pair once.
    let mut jobs: BTreeSet<(usize, &str)> = BTreeSet::new();
    for (file_idx, file) in files.iter().enumerate() {
        if !selected(&file.path) {
            continue;
        }
        if suppressions[file_idx].ignore_file {
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
    let mut suppressed = 0;
    // Files are independent, so they are checked in parallel; results come back in job order.
    let jobs: Vec<(usize, &str)> = jobs.into_iter().collect();
    let results: Vec<(Vec<Violation>, usize)> = jobs
        .par_iter()
        .map(|&(file_idx, id)| {
            let mut found = Vec::new();
            check_file(&files[file_idx], &rules[id], &ctx, &mut found);
            let before = found.len();
            found.retain(|v| !suppressions[file_idx].hides(v.line));
            let hidden = before - found.len();
            (found, hidden)
        })
        .collect();
    for (&(file_idx, _), (found, hidden)) in jobs.iter().zip(results) {
        let file = &files[file_idx];
        if checked.insert(file.path.as_str()) && file.has_syntax_errors() {
            eprintln!("prlint: warning: {} has syntax errors, results may be incomplete", file.path);
        }
        suppressed += hidden;
        violations.extend(found);
    }

    // Registry entries that nothing uses are reported in the registry file, from every file in scope.
    for (id, rule) in rules.iter().filter(|(_, r)| r.literal.as_ref().is_some_and(|l| l.report_unused)) {
        let Some(registry) = rule.literal.as_ref().map(|l| l.file.as_str()) else { continue };
        let Some(registry_idx) = files.iter().position(|f| f.path == registry) else { continue };
        if !selected(registry) || suppressions[registry_idx].ignore_file {
            continue;
        }
        let users = files.iter().filter(|f| {
            !f.language.is_data()
                && rule.rule.applies_to(f.language)
                && scopes.iter().any(|s| s.rules.iter().any(|r| r == id) && s.matches(&f.path))
        });
        checked.insert(registry);
        let mut unused = unused_registry_entries(rule, users, &ctx);
        unused.retain(|v| {
            let hide = suppressions[registry_idx].hides(v.line);
            suppressed += usize::from(hide);
            !hide
        });
        violations.extend(unused);
    }

    // File-name rules look at every file, not only the ones we can parse.
    let mut name_jobs: BTreeSet<(&str, &str)> = BTreeSet::new();
    for path in &all_paths {
        if !selected(path) {
            continue;
        }
        if ignored.contains(path.as_str()) {
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
    // What was hidden stays visible in the summary, so suppressions can be audited.
    let mut notes = Vec::new();
    if suppressed > 0 {
        notes.push(format!("{suppressed} hidden by prlint-ignore-next-line"));
    }
    let ignored_in_scope = ignored
        .iter()
        .filter(|p| selected(p))
        .filter(|p| scopes.iter().any(|s| s.matches(p)))
        .count();
    if ignored_in_scope > 0 {
        notes.push(format!("{ignored_in_scope} file(s) skipped by prlint-ignore-file"));
    }
    let notes = if notes.is_empty() { String::new() } else { format!(" [{}]", notes.join(", ")) };
    if violations.is_empty() {
        println!("prlint: no violations ({} files checked){notes}", checked.len());
    } else {
        println!(
            "prlint: {} violation(s) in {} file(s){notes}",
            violations.len(),
            violations.iter().map(|v| &v.path).collect::<BTreeSet<_>>().len()
        );
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
