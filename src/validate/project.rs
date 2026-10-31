//! Checks that need the project, not just the config: scopes that match nothing, missing files, rules a language cannot run.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};

use crate::check::{ClassIndex, normalize_file};
use crate::config::Config;
use crate::data::{Doc, PathPattern};
use crate::scope::CompiledScope;
use crate::source;

use super::report::{Report, RuleSummary};

/// Looks at the project on disk: which files each rule really runs on, and classes it names.
pub(super) fn check_against_project(
    config: &Config,
    config_path: &Path,
    referenced: &BTreeSet<String>,
    report: &mut Report,
) -> Result<Vec<RuleSummary>> {
    let root = std::path::absolute(config_path)?
        .parent()
        .map(Path::to_path_buf)
        .context("config file has no parent directory")?;
    let files = source::load_all(&root)?;
    let index = ClassIndex::build(&files);
    let all_paths = source::list_all(&root)?;
    let scopes = config.scopes.iter().map(CompiledScope::new).collect::<Result<Vec<_>>>()?;

    let docs: std::collections::BTreeMap<String, Doc> =
        files.iter().filter(|f| f.language.is_data()).map(|f| (f.path.clone(), Doc::build(f))).collect();

    let mut summaries = Vec::new();
    for (id, rule) in &config.rules {
        if let Some(reference) = &rule.same_keys_as
            && !docs.contains_key(&normalize_file(reference))
        {
            report.error(
                &format!("rules.{id}.same_keys_as"),
                format!("`{reference}` is not a JSON, YAML or TOML file in the project"),
                Some(format!("the path is relative to {}; files ignored by .gitignore are skipped", root.display())),
            );
        }
        if let Some(arg) = &rule.literal_argument {
            let at = format!("rules.{id}.literal_argument.registry");
            match docs.get(&normalize_file(&arg.registry.file)) {
                None => report.error(
                    &format!("{at}.file"),
                    format!("`{}` is not a JSON, YAML or TOML file in the project", arg.registry.file),
                    Some(format!("the path is relative to {}; files ignored by .gitignore are skipped", root.display())),
                ),
                Some(doc) => {
                    if let Ok(pattern) = PathPattern::parse(&arg.registry.path)
                        && doc.registry(&pattern, arg.registry.use_).is_empty()
                    {
                        report.warning(
                            &format!("{at}.path"),
                            format!("`{}` selects no entries in {}, so every literal would be reported", arg.registry.path, arg.registry.file),
                            Some("`use: value` and `use: path` only count scalars; check the path against the file's structure".into()),
                        );
                    }
                }
            }
        }
        let mut matched: BTreeSet<&str> = BTreeSet::new();
        let mut per_scope = Vec::new();
        for (i, scope) in scopes.iter().enumerate().filter(|(_, s)| s.rules.contains(id)) {
            let hits: Vec<&str> = all_paths
                .iter()
                .filter(|p| rule.applies_to_path(p))
                .filter(|p| scope.matches(p))
                .map(String::as_str)
                .collect();
            if hits.is_empty() {
                report.warning(
                    &format!("scopes[{i}]"),
                    format!(
                        "rule `{id}` matches no {}files here, so it has nothing to check",
                        if rule.language.is_empty() { String::new() } else { format!("{} ", rule.language_label()) }
                    ),
                    Some(format!(
                        "include/exclude globs are relative to {}; `*` stays in one directory, `**` crosses directories",
                        root.display()
                    )),
                );
            }
            per_scope.push((i, hits.len()));
            matched.extend(hits);
        }
        if !referenced.contains(id) {
            report.warning(
                &format!("rules.{id}"),
                "no scope uses this rule, so it never runs",
                Some(format!("add `{id}` to the `rules` list of an entry under `scopes`")),
            );
        }
        if let Some(catch) = &rule.forbid_catch
            && !index.defines(&catch.subclass_of)
        {
            report.warning(
                &format!("rules.{id}.forbid_catch.subclass_of"),
                format!("no class named `{}` is defined in the project", catch.subclass_of),
                Some("the rule will only match `except` clauses naming it directly; check the spelling".into()),
            );
        }

        let forbids = rule.describe_checks();
        summaries.push(RuleSummary {
            id: id.clone(),
            language: rule.language_label(),
            forbids,
            files: matched.len(),
            scopes: per_scope,
        });
    }
    Ok(summaries)
}
