use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use globset::{Glob, GlobBuilder};
use serde_yaml_ng::{Mapping, Value};

use crate::check::ClassIndex;
use crate::config::Config;
use crate::scope::CompiledScope;
use crate::source;

const TOP_KEYS: &[(&str, &str)] = &[
    ("rules", "mapping of rule id to rule definition"),
    ("scopes", "list of entries that say which files each rule runs on"),
    ("aliases", "optional; import prefix to project path, e.g. `\"@/\": \"src/\"`"),
];
const RULE_KEYS: &[(&str, &str)] = &[
    ("language", "`python`, `javascript` or `typescript`, or a list of them; required with forbid_* checks"),
    ("forbid_raise", "list of exception names that must not be raised (`throw` in JavaScript and TypeScript)"),
    ("forbid_call", "list of call patterns that must not appear; `*` matches anything, e.g. `*.one`"),
    ("forbid_catch", "python only; `subclass_of: Name` forbids catching Name or any class inheriting from it"),
    ("forbid_import", "JS/TS; module specifiers that must not be imported; relative and aliased imports match by project path"),
    ("except_import", "JS/TS; imports that are fine although they match forbid_import"),
    ("forbid_element", "JS/TS; JSX element names that must not be used, e.g. `button`"),
    ("forbid_property", "JS/TS; object-literal property names that must not appear"),
    ("property_value", "JS/TS; `names`, plus `forbid_literal` and/or `array_starts_with`, and optionally `in_call`"),
    ("within_property", "JS/TS; only count call, raise and element hits inside a property with one of these names"),
    ("require_filename", "list of file name globs; any matched file whose name matches none is a violation"),
    ("rationale", "required; why the rule exists, shown with every violation"),
    ("fix", "required; how to comply, shown with every violation"),
];
const SCOPE_KEYS: &[(&str, &str)] = &[
    ("rules", "required; list of rule ids that run on the matched files"),
    ("include", "required; list of path globs relative to the config file's directory"),
    ("exclude", "optional; list of path globs to leave out"),
];
const LANGUAGES: &[&str] = &["python", "javascript", "typescript"];

struct Diagnostic {
    path: String,
    message: String,
    help: Option<String>,
}

#[derive(Default)]
struct Report {
    errors: Vec<Diagnostic>,
    warnings: Vec<Diagnostic>,
}

impl Report {
    fn error(&mut self, path: &str, message: impl Into<String>, help: Option<String>) {
        self.errors.push(Diagnostic { path: path.into(), message: message.into(), help });
    }

    fn warning(&mut self, path: &str, message: impl Into<String>, help: Option<String>) {
        self.warnings.push(Diagnostic { path: path.into(), message: message.into(), help });
    }
}

/// Validates the config file and prints the verdict. Returns whether it is valid.
pub fn run(config_path: &Path) -> Result<bool> {
    let name = config_path.display();
    let text = std::fs::read_to_string(config_path).with_context(|| format!("cannot read {name}"))?;

    let value: Value = match serde_yaml_ng::from_str(&text) {
        Ok(value) => value,
        Err(err) => {
            println!("{name}: INVALID (1 error)\n");
            let at = err.location().map_or(String::new(), |l| format!(" (line {}, column {})", l.line(), l.column()));
            println!("error: not valid YAML{at}");
            println!("  {err}");
            return Ok(false);
        }
    };

    let mut report = Report::default();
    let referenced = validate_structure(&value, &mut report);

    // The typed loader is what `check` uses, so it has the final say.
    let config = if report.errors.is_empty() {
        match Config::parse(&text) {
            Ok(config) => Some(config),
            Err(err) => {
                report.error("", format!("{err:#}"), None);
                None
            }
        }
    } else {
        None
    };

    let mut summary = Vec::new();
    if let Some(config) = &config {
        summary = check_against_project(config, config_path, &referenced, &mut report)?;
    }

    print_report(&name.to_string(), &report, config.as_ref(), &summary);
    Ok(report.errors.is_empty())
}

struct RuleSummary {
    id: String,
    language: String,
    forbids: Vec<String>,
    files: usize,
    scopes: Vec<(usize, usize)>,
}

/// Looks at the project on disk: which files each rule really runs on, and classes it names.
fn check_against_project(
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

    let mut summaries = Vec::new();
    for (id, rule) in &config.rules {
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

fn print_report(name: &str, report: &Report, config: Option<&Config>, summary: &[RuleSummary]) {
    let warnings = match report.warnings.len() {
        0 => String::new(),
        n => format!(", {n} warning(s)"),
    };
    if report.errors.is_empty() {
        let (rules, scopes) = config.map_or((0, 0), |c| (c.rules.len(), c.scopes.len()));
        println!("{name}: VALID ({rules} rule(s), {scopes} scope(s){warnings})");
    } else {
        println!("{name}: INVALID ({} error(s){warnings})", report.errors.len());
    }
    println!();
    for (label, items) in [("error", &report.errors), ("warning", &report.warnings)] {
        for d in items {
            let at = if d.path.is_empty() { String::new() } else { format!(" at `{}`", d.path) };
            println!("{label}{at}: {}", d.message);
            if let Some(help) = &d.help {
                for (i, line) in help.lines().enumerate() {
                    println!("  {}{line}", if i == 0 { "help: " } else { "      " });
                }
            }
        }
    }
    if !report.errors.is_empty() {
        println!("\nFix the errors above and run `prlint validate` again.");
        return;
    }
    if !report.warnings.is_empty() {
        println!();
    }
    for rule in summary {
        let scopes: Vec<String> = rule.scopes.iter().map(|(i, n)| format!("scopes[{i}]: {n} file(s)")).collect();
        println!("rule {} ({})", rule.id, rule.language);
        println!("  forbids: {}", rule.forbids.join("; "));
        println!("  runs on {} file(s) {}", rule.files, if scopes.is_empty() { "(no scope)".into() } else { format!("[{}]", scopes.join(", ")) });
    }
    println!("\nNext: `prlint check` to find violations, `prlint explain <rule>` to see one rule in full.");
}

// ---------------------------------------------------------------------------------------------
// Structural validation of the raw YAML. It reports every problem it can find in one pass,
// with the path to the offending key, instead of stopping at the first one like serde does.
// ---------------------------------------------------------------------------------------------

/// Returns the set of rule ids that scopes refer to.
fn validate_structure(root: &Value, report: &mut Report) -> BTreeSet<String> {
    let mut referenced = BTreeSet::new();
    let Some(top) = expect_mapping(root, "", "a mapping with the keys `rules` and `scopes`", report) else {
        return referenced;
    };
    check_keys(top, "", TOP_KEYS, report);
    if let Some(aliases) = top.get("aliases") {
        validate_aliases(aliases, report);
    }

    let mut rule_ids = BTreeSet::new();
    match top.get("rules") {
        None => report.error("", "missing required key `rules`", Some("add `rules:` with at least one rule".into())),
        Some(rules) => rule_ids = validate_rules(rules, report),
    }
    match top.get("scopes") {
        None => report.error(
            "",
            "missing required key `scopes`",
            Some("add `scopes:` with a list of entries, each with `rules` and `include`".into()),
        ),
        Some(scopes) => referenced = validate_scopes(scopes, &rule_ids, report),
    }
    referenced
}

fn validate_rules(rules: &Value, report: &mut Report) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let Some(map) = expect_mapping(rules, "rules", "a mapping of rule id to rule definition", report) else {
        return ids;
    };
    if map.is_empty() {
        report.error("rules", "no rules are defined", Some("add at least one rule under `rules:`".into()));
    }
    for (key, value) in map {
        let Some(id) = key.as_str() else {
            report.error("rules", format!("rule ids must be strings, found {}", describe(key)), None);
            continue;
        };
        ids.insert(id.to_string());
        validate_rule(&format!("rules.{id}"), value, report);
    }
    ids
}

fn validate_rule(path: &str, value: &Value, report: &mut Report) {
    let Some(rule) = expect_mapping(value, path, "a mapping with a check, `rationale` and `fix` (and `language` for forbid_* checks)", report)
    else {
        return;
    };
    check_keys(rule, path, RULE_KEYS, report);

    let has_code_checks = [
        "forbid_raise",
        "forbid_call",
        "forbid_catch",
        "forbid_import",
        "forbid_element",
        "forbid_property",
        "property_value",
    ]
    .iter()
    .any(|k| rule.get(*k).is_some());
    let languages = match rule.get("language") {
        None if has_code_checks => {
            report.error(
                path,
                "missing required key `language`",
                Some(format!("add `language: {}`; it is required by every check except require_filename", LANGUAGES[0])),
            );
            Vec::new()
        }
        None => Vec::new(),
        Some(value) => validate_languages(value, &format!("{path}.language"), report),
    };
    for (key, hint) in [("rationale", "explain why the rule exists"), ("fix", "explain how to comply")] {
        match rule.get(key) {
            None => report.error(path, format!("missing required key `{key}`"), Some(format!("add `{key}:` to {hint}; it is returned with every violation"))),
            Some(Value::String(s)) if !s.trim().is_empty() => {}
            Some(other) => report.error(&format!("{path}.{key}"), format!("expected non-empty text, found {}", describe(other)), None),
        }
    }

    let mut checks = 0;
    if let Some(value) = rule.get("forbid_raise") {
        checks += 1;
        string_list(value, &format!("{path}.forbid_raise"), report);
    }
    if let Some(value) = rule.get("forbid_call") {
        checks += 1;
        let call_path = format!("{path}.forbid_call");
        for (i, pattern) in string_list(value, &call_path, report).iter().enumerate() {
            if let Err(err) = Glob::new(pattern) {
                report.error(&format!("{call_path}[{i}]"), format!("invalid pattern `{pattern}`: {}", err.kind()), None);
            }
        }
    }
    if let Some(value) = rule.get("require_filename") {
        checks += 1;
        let name_path = format!("{path}.require_filename");
        for (i, pattern) in string_list(value, &name_path, report).iter().enumerate() {
            if let Err(err) = Glob::new(pattern) {
                report.error(&format!("{name_path}[{i}]"), format!("invalid glob `{pattern}`: {}", err.kind()), None);
            }
        }
    }
    if let Some(value) = rule.get("forbid_catch") {
        checks += 1;
        if languages.iter().any(|l| l != "python") {
            report.error(
                &format!("{path}.forbid_catch"),
                "forbid_catch is only supported for python",
                Some("JavaScript and TypeScript catch clauses have no exception type; use `language: python` or remove forbid_catch".into()),
            );
        }
        let catch_path = format!("{path}.forbid_catch");
        if let Some(map) = expect_mapping(value, &catch_path, "a mapping like `{ subclass_of: SomeError }`", report) {
            check_keys(map, &catch_path, &[("subclass_of", "required; class name whose subclasses may not be caught")], report);
            match map.get("subclass_of") {
                None => report.error(&catch_path, "missing required key `subclass_of`", Some("write `forbid_catch: { subclass_of: ClassName }`".into())),
                Some(Value::String(s)) if !s.trim().is_empty() => {}
                Some(other) => report.error(&format!("{catch_path}.subclass_of"), format!("expected a class name, found {}", describe(other)), None),
            }
        }
    }
    validate_ecmascript_keys(rule, path, &languages, &mut checks, report);
    if checks == 0 {
        report.error(
            path,
            "the rule has no checks, so it can never report anything",
            Some("add at least one check: forbid_raise, forbid_call, forbid_catch, forbid_import, forbid_element, forbid_property, property_value or require_filename".into()),
        );
    }
}

fn validate_aliases(value: &Value, report: &mut Report) {
    let Some(map) = expect_mapping(value, "aliases", "a mapping like `\"@/\": \"src/\"`", report) else { return };
    for (key, target) in map {
        let (Some(prefix), Some(target)) = (key.as_str(), target.as_str()) else {
            report.error("aliases", "aliases map a prefix string to a project path string", None);
            continue;
        };
        let path = format!("aliases.{prefix}");
        if prefix.is_empty() || target.is_empty() {
            report.error(&path, "the prefix and the path must not be empty", None);
        } else if prefix.ends_with('/') != target.ends_with('/') {
            report.error(
                &path,
                format!("`{prefix}` and `{target}` must both end with `/`, or neither"),
                Some("the prefix is replaced by the path, so `\"@/\": \"src/\"` turns `@/api` into `src/api`".into()),
            );
        }
    }
}

/// The JS/TS-only keys: forbid_import, except_import, forbid_element, forbid_property, property_value, within_property.
fn validate_ecmascript_keys(rule: &Mapping, path: &str, languages: &[String], checks: &mut usize, report: &mut Report) {
    const KEYS: &[&str] = &[
        "forbid_import",
        "except_import",
        "forbid_element",
        "forbid_property",
        "property_value",
        "within_property",
    ];
    let present: Vec<&str> = KEYS.iter().copied().filter(|k| rule.get(*k).is_some()).collect();
    if present.is_empty() {
        return;
    }
    if languages.iter().any(|l| l == "python") {
        report.error(
            &format!("{path}.{}", present[0]),
            format!("{} {} only supported for JavaScript and TypeScript", present.join(", "), if present.len() == 1 { "is" } else { "are" }),
            Some("set `language: [javascript, typescript]`, or split the Python checks into their own rule".into()),
        );
    }
    for key in ["forbid_import", "except_import"] {
        if let Some(value) = rule.get(key) {
            if key == "forbid_import" {
                *checks += 1;
            }
            glob_list(value, &format!("{path}.{key}"), report);
        }
    }
    if rule.get("except_import").is_some() && rule.get("forbid_import").is_none() {
        report.error(&format!("{path}.except_import"), "except_import has nothing to except without forbid_import", Some("add forbid_import, or remove except_import".into()));
    }
    if let Some(value) = rule.get("forbid_element") {
        *checks += 1;
        let element_path = format!("{path}.forbid_element");
        for (i, pattern) in string_list(value, &element_path, report).iter().enumerate() {
            if let Err(err) = Glob::new(pattern) {
                report.error(&format!("{element_path}[{i}]"), format!("invalid pattern `{pattern}`: {}", err.kind()), None);
            }
        }
    }
    if let Some(value) = rule.get("forbid_property") {
        *checks += 1;
        string_list(value, &format!("{path}.forbid_property"), report);
    }
    if let Some(value) = rule.get("within_property") {
        string_list(value, &format!("{path}.within_property"), report);
        if ["forbid_call", "forbid_raise", "forbid_element"].iter().all(|k| rule.get(*k).is_none()) {
            report.error(
                &format!("{path}.within_property"),
                "within_property has nothing to narrow",
                Some("it applies to forbid_call, forbid_raise and forbid_element".into()),
            );
        }
    }
    if let Some(value) = rule.get("property_value") {
        *checks += 1;
        validate_property_value(value, &format!("{path}.property_value"), report);
    }
}

fn validate_property_value(value: &Value, path: &str, report: &mut Report) {
    const KEYS: &[(&str, &str)] = &[
        ("names", "required; property name or list of names to look at"),
        ("in_call", "optional; only objects passed directly to one of these calls, e.g. `toast.add`"),
        ("forbid_literal", "true: the value must not be a string literal"),
        ("array_starts_with", "an array-literal value must start with this element, e.g. `accountID`"),
    ];
    let Some(map) = expect_mapping(value, path, "a mapping with `names` and a constraint", report) else { return };
    check_keys(map, path, KEYS, report);
    match map.get("names") {
        None => report.error(path, "missing required key `names`", Some("add `names: [queryKey]`".into())),
        Some(v) => {
            string_or_list(v, &format!("{path}.names"), report);
        }
    }
    if let Some(v) = map.get("in_call") {
        string_or_list(v, &format!("{path}.in_call"), report);
    }
    let literal = match map.get("forbid_literal") {
        None => false,
        Some(Value::Bool(b)) => *b,
        Some(other) => {
            report.error(&format!("{path}.forbid_literal"), format!("expected true or false, found {}", describe(other)), None);
            false
        }
    };
    let starts = match map.get("array_starts_with") {
        None => false,
        Some(Value::String(s)) if !s.trim().is_empty() => true,
        Some(other) => {
            report.error(&format!("{path}.array_starts_with"), format!("expected the text of the first element, found {}", describe(other)), None);
            false
        }
    };
    if !literal && !starts && !report.errors.iter().any(|d| d.path.starts_with(path)) {
        report.error(path, "the property_value has no constraint", Some("add `forbid_literal: true` or `array_starts_with: <element>`".into()));
    }
}

/// A string or a list of strings.
fn string_or_list(value: &Value, path: &str, report: &mut Report) -> Vec<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => vec![s.clone()],
        Value::Sequence(_) => string_list(value, path, report),
        other => {
            report.error(path, format!("expected a name or a list of names, found {}", describe(other)), None);
            Vec::new()
        }
    }
}

/// `language: python` or `language: [javascript, typescript]`. Returns the valid names.
fn validate_languages(value: &Value, path: &str, report: &mut Report) -> Vec<String> {
    let names: Vec<(String, &Value)> = match value {
        Value::Sequence(items) if items.is_empty() => {
            report.error(path, "the list is empty", Some(format!("name at least one language: {}", LANGUAGES.join(", "))));
            return Vec::new();
        }
        Value::Sequence(items) => items.iter().map(|v| (path.to_string(), v)).collect(),
        other => vec![(path.to_string(), other)],
    };
    let mut valid = Vec::new();
    for (i, (_, item)) in names.iter().enumerate() {
        let at = if matches!(value, Value::Sequence(_)) { format!("{path}[{i}]") } else { path.to_string() };
        match item {
            Value::String(lang) if LANGUAGES.contains(&lang.as_str()) => valid.push(lang.clone()),
            Value::String(lang) => report.error(
                &at,
                format!("unsupported language `{lang}`"),
                Some(with_suggestion(lang, LANGUAGES.iter().copied(), &format!("supported languages: {}", LANGUAGES.join(", ")))),
            ),
            other => report.error(&at, format!("expected a language name, found {}", describe(other)), None),
        }
    }
    valid
}

fn validate_scopes(scopes: &Value, rule_ids: &BTreeSet<String>, report: &mut Report) -> BTreeSet<String> {
    let mut referenced = BTreeSet::new();
    let Some(list) = scopes.as_sequence() else {
        report.error(
            "scopes",
            format!("expected a list of scope entries, found {}", describe(scopes)),
            Some("each entry starts with `- rules: [...]` and has an `include:` list".into()),
        );
        return referenced;
    };
    if list.is_empty() {
        report.error("scopes", "no scopes are defined, so no rule would run", Some("add an entry with `rules` and `include`".into()));
    }
    for (i, entry) in list.iter().enumerate() {
        let path = format!("scopes[{i}]");
        let Some(map) = expect_mapping(entry, &path, "a mapping with `rules` and `include`", report) else { continue };
        check_keys(map, &path, SCOPE_KEYS, report);

        match map.get("rules") {
            None => report.error(&path, "missing required key `rules`", Some("list the rule ids this entry applies, e.g. `rules: [my-rule]`".into())),
            Some(value) => {
                for (j, id) in string_list(value, &format!("{path}.rules"), report).iter().enumerate() {
                    if rule_ids.contains(id) {
                        referenced.insert(id.clone());
                    } else {
                        let known = if rule_ids.is_empty() { "no rules are defined".to_string() } else { format!("defined rules: {}", rule_ids.iter().cloned().collect::<Vec<_>>().join(", ")) };
                        report.error(
                            &format!("{path}.rules[{j}]"),
                            format!("unknown rule `{id}`"),
                            Some(with_suggestion(id, rule_ids.iter().map(String::as_str), &known)),
                        );
                    }
                }
            }
        }
        match map.get("include") {
            None => report.error(&path, "missing required key `include`", Some("add `include: [\"src/**\"]` (paths relative to the config file)".into())),
            Some(value) => glob_list(value, &format!("{path}.include"), report),
        }
        if let Some(value) = map.get("exclude") {
            glob_list(value, &format!("{path}.exclude"), report);
        }
    }
    referenced
}

fn glob_list(value: &Value, path: &str, report: &mut Report) {
    let patterns = string_list(value, path, report);
    for (i, pattern) in patterns.iter().enumerate() {
        if let Err(err) = GlobBuilder::new(pattern).literal_separator(true).build() {
            report.error(&format!("{path}[{i}]"), format!("invalid glob `{pattern}`: {}", err.kind()), None);
        }
    }
}

/// A non-empty list of non-empty strings. Reports problems and returns the usable entries.
fn string_list(value: &Value, path: &str, report: &mut Report) -> Vec<String> {
    let Some(items) = value.as_sequence() else {
        report.error(path, format!("expected a list of strings, found {}", describe(value)), Some("write it as `[a, b]` or one `- item` per line".into()));
        return Vec::new();
    };
    if items.is_empty() {
        report.error(path, "the list is empty", Some("add at least one entry or remove the key".into()));
    }
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match item.as_str() {
            Some(s) if !s.trim().is_empty() => out.push(s.to_string()),
            _ => report.error(&format!("{path}[{i}]"), format!("expected a non-empty string, found {}", describe(item)), Some("quote values that contain `*`, `:` or `{`".into())),
        }
    }
    out
}

fn expect_mapping<'a>(value: &'a Value, path: &str, expected: &str, report: &mut Report) -> Option<&'a Mapping> {
    let map = value.as_mapping();
    if map.is_none() {
        report.error(path, format!("expected {expected}, found {}", describe(value)), None);
    }
    map
}

fn check_keys(map: &Mapping, path: &str, allowed: &[(&str, &str)], report: &mut Report) {
    for key in map.keys() {
        let Some(name) = key.as_str() else {
            report.error(path, format!("keys must be strings, found {}", describe(key)), None);
            continue;
        };
        if allowed.iter().any(|(k, _)| *k == name) {
            continue;
        }
        let valid = allowed.iter().map(|(k, d)| format!("  {k}: {d}")).collect::<Vec<_>>().join("\n");
        let at = if path.is_empty() { name.to_string() } else { format!("{path}.{name}") };
        let guess = closest(name, allowed.iter().map(|(k, _)| *k))
            .map_or(String::new(), |near| format!("did you mean `{near}`?\n"));
        report.error(&at, format!("unknown key `{name}`"), Some(format!("{guess}valid keys here:\n{valid}")));
    }
}

fn with_suggestion<'a>(word: &str, candidates: impl Iterator<Item = &'a str>, otherwise: &str) -> String {
    match closest(word, candidates) {
        Some(near) => format!("did you mean `{near}`? ({otherwise})"),
        None => otherwise.to_string(),
    }
}

fn closest<'a>(word: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (word.len() / 3).max(2);
    candidates
        .map(|c| (edit_distance(word, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != *cb);
            let next = (row[j] + 1).min(row[j + 1] + 1).min(prev + cost);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Null => "an empty value",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Sequence(_) => "a list",
        Value::Mapping(_) => "a mapping",
        Value::Tagged(_) => "a tagged value",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
rules:
  no-http:
    language: python
    forbid_raise: [HTTPException]
    rationale: why
    fix: how
scopes:
  - rules: [no-http]
    include: ["src/**"]
"#;

    fn errors(yaml: &str) -> Vec<(String, String)> {
        let value: Value = serde_yaml_ng::from_str(yaml).unwrap();
        let mut report = Report::default();
        validate_structure(&value, &mut report);
        report.errors.into_iter().map(|d| (d.path, d.message)).collect()
    }

    fn has(errors: &[(String, String)], path: &str, fragment: &str) -> bool {
        errors.iter().any(|(p, m)| p == path && m.contains(fragment))
    }

    #[test]
    fn accepts_a_good_config() {
        assert!(errors(GOOD).is_empty());
    }

    #[test]
    fn reports_every_problem_in_one_pass() {
        let yaml = GOOD
            .replace("forbid_raise", "forbid_rase")
            .replace("    fix: how\n", "")
            .replace("rules: [no-http]", "rules: [no-htp]");
        let found = errors(&yaml);
        assert!(has(&found, "rules.no-http.forbid_rase", "unknown key"), "{found:?}");
        assert!(has(&found, "rules.no-http", "missing required key `fix`"), "{found:?}");
        assert!(has(&found, "rules.no-http", "no checks"), "{found:?}");
        assert!(has(&found, "scopes[0].rules[0]", "unknown rule `no-htp`"), "{found:?}");
    }

    #[test]
    fn flags_wrong_types_and_bad_globs() {
        let yaml = GOOD.replace("[HTTPException]", "HTTPException").replace("\"src/**\"", "\"src/[\"");
        let found = errors(&yaml);
        assert!(has(&found, "rules.no-http.forbid_raise", "expected a list"), "{found:?}");
        assert!(has(&found, "scopes[0].include[0]", "invalid glob"), "{found:?}");
    }

    #[test]
    fn rejects_unsupported_language_with_suggestion() {
        let found = errors(&GOOD.replace("language: python", "language: pyhton"));
        assert!(has(&found, "rules.no-http.language", "unsupported language"), "{found:?}");
    }

    #[test]
    fn filename_rule_needs_no_language() {
        let yaml = r#"
rules:
  names:
    require_filename: ["test_*.py", conftest.py]
    rationale: r
    fix: f
scopes:
  - rules: [names]
    include: ["tests/**"]
"#;
        assert!(errors(yaml).is_empty(), "{:?}", errors(yaml));
    }

    #[test]
    fn code_checks_still_need_a_language() {
        let found = errors(&GOOD.replace("    language: python\n", ""));
        assert!(has(&found, "rules.no-http", "missing required key `language`"), "{found:?}");
    }

    #[test]
    fn language_may_be_a_list() {
        let yaml = GOOD.replace("language: python", "language: [javascript, typescript]");
        assert!(errors(&yaml).is_empty(), "{:?}", errors(&yaml));
        let found = errors(&GOOD.replace("language: python", "language: [typescript, typscript]"));
        assert!(has(&found, "rules.no-http.language[1]", "unsupported language"), "{found:?}");
    }

    #[test]
    fn forbid_catch_is_rejected_outside_python() {
        let yaml = GOOD.replace("language: python", "language: typescript").replace(
            "forbid_raise: [HTTPException]",
            "forbid_catch: { subclass_of: AppError }",
        );
        let found = errors(&yaml);
        assert!(has(&found, "rules.no-http.forbid_catch", "only supported for python"), "{found:?}");
        let ok = GOOD.replace("forbid_raise: [HTTPException]", "forbid_catch: { subclass_of: AppError }");
        assert!(errors(&ok).is_empty());
    }

    const ES_RULE: &str = r#"
rules:
  r:
    language: typescript
    rationale: why
    fix: how
scopes:
  - rules: [r]
    include: ["src/**"]
"#;

    fn es(extra: &str) -> Vec<(String, String)> {
        errors(&ES_RULE.replace("    fix: how\n", &format!("    fix: how\n{extra}")))
    }

    #[test]
    fn accepts_the_javascript_checks() {
        for extra in [
            "    forbid_import: [\"@base-ui/react/**\"]\n    except_import: [\"src/ui/**\"]\n",
            "    forbid_element: [button, input]\n",
            "    forbid_property: [onError]\n",
            "    forbid_call: [toast.add]\n    within_property: [onError]\n",
            "    property_value: { names: queryKey, array_starts_with: accountID }\n",
            "    property_value: { names: [title], in_call: toast.add, forbid_literal: true }\n",
        ] {
            assert!(es(extra).is_empty(), "{extra}: {:?}", es(extra));
        }
    }

    #[test]
    fn rejects_bad_javascript_checks() {
        let found = es("    except_import: [\"src/**\"]\n");
        assert!(has(&found, "rules.r.except_import", "nothing to except"), "{found:?}");
        let found = es("    forbid_import: [\"src/[\"]\n");
        assert!(has(&found, "rules.r.forbid_import[0]", "invalid glob"), "{found:?}");
        let found = es("    within_property: [onError]\n    forbid_property: [x]\n");
        assert!(has(&found, "rules.r.within_property", "nothing to narrow"), "{found:?}");
        let found = es("    property_value: { names: queryKey }\n");
        assert!(has(&found, "rules.r.property_value", "no constraint"), "{found:?}");
        let found = es("    property_value: { array_starts_with: a }\n");
        assert!(has(&found, "rules.r.property_value", "missing required key `names`"), "{found:?}");
        let found = es("    property_value: { names: x, forbid_litral: true }\n");
        assert!(has(&found, "rules.r.property_value.forbid_litral", "unknown key"), "{found:?}");
    }

    #[test]
    fn javascript_checks_are_rejected_for_python() {
        let yaml = ES_RULE.replace("language: typescript", "language: [python, typescript]").replace(
            "    fix: how\n",
            "    fix: how\n    forbid_import: [x]\n",
        );
        let found = errors(&yaml);
        assert!(has(&found, "rules.r.forbid_import", "only supported for JavaScript and TypeScript"), "{found:?}");
    }

    #[test]
    fn aliases_must_agree_on_the_trailing_slash() {
        let yaml = format!("aliases:\n  \"@/\": src\n{GOOD}");
        let found = errors(&yaml);
        assert!(has(&found, "aliases.@/", "must both end with"), "{found:?}");
        assert!(errors(&format!("aliases:\n  \"@/\": src/\n{GOOD}")).is_empty());
    }

    #[test]
    fn suggests_close_names() {
        assert_eq!(closest("forbid_rase", ["forbid_raise", "fix"].into_iter()), Some("forbid_raise"));
        assert_eq!(closest("zzzzzz", ["forbid_raise"].into_iter()), None);
    }

    #[test]
    fn missing_sections_are_reported() {
        let found = errors("rules: {}\n");
        assert!(has(&found, "rules", "no rules"), "{found:?}");
        assert!(has(&found, "", "missing required key `scopes`"), "{found:?}");
    }
}
