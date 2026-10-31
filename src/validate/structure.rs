//! Structural validation of the raw YAML: top-level sections, rules and scopes. It reports every
//! problem it can find in one pass, with the path to the offending key, instead of stopping at the
//! first one like serde does.

use std::collections::BTreeSet;

use serde_yaml_ng::Value;

use super::keys::{SCOPE_KEYS, TOP_KEYS};
use super::report::Report;
use super::rule::validate_rule;
use super::util::{check_keys, describe, expect_mapping, glob_list, string_list, with_suggestion};

/// Returns the set of rule ids that scopes refer to.
pub(super) fn validate_structure(root: &Value, report: &mut Report) -> BTreeSet<String> {
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

pub(super) fn validate_rules(rules: &Value, report: &mut Report) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let Some(map) = expect_mapping(rules, "rules", "a mapping of rule id to rule definition", report) else {
        return ids;
    };
    if map.is_empty() {
        report.warning("rules", "no rules are defined yet, so nothing is enforced", Some("add a rule under `rules:`".into()));
    }
    for (key, value) in map {
        let Some(id) = key.as_str() else {
            report.error("rules", format!("rule ids must be strings, found {}", describe(key)), None);
            continue;
        };
        if let Some(problem) = crate::config::id_problem(id) {
            report.error(&format!("rules.{id}"), format!("the rule id {problem}"), None);
        }
        ids.insert(id.to_string());
        validate_rule(&format!("rules.{id}"), value, report);
    }
    ids
}

pub(super) fn validate_scopes(scopes: &Value, rule_ids: &BTreeSet<String>, report: &mut Report) -> BTreeSet<String> {
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
        report.warning("scopes", "no scopes are defined, so no rule would run", Some("add an entry with `rules` and `include`".into()));
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

pub(super) fn validate_aliases(value: &Value, report: &mut Report) {
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
