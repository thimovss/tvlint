//! Validation of the keys that read JSON, YAML and TOML files.

use serde_yaml_ng::{Mapping, Value};

use crate::config::Language;
use crate::data::PathPattern;

use super::code_keys::validate_literal_argument;
use super::report::Report;
use super::util::{describe, string_or_list};

/// Key paths of data files: a dotted path with `*` and `**`, checked the way `check` will read it.
pub(super) fn path_patterns(value: &Value, path: &str, report: &mut Report) {
    for (i, pattern) in string_or_list(value, path, report).iter().enumerate() {
        if let Err(err) = PathPattern::parse(pattern) {
            report.error(
                &format!("{path}[{i}]"),
                format!("invalid key path: {err}"),
                Some("write keys separated by dots; `*` is any one key or list index, `**` any depth, and a key containing a dot goes in double quotes".into()),
            );
        }
    }
}

pub(super) fn validate_data_keys(rule: &Mapping, path: &str, languages: &[String], checks: &mut usize, report: &mut Report) {
    let data = languages.iter().filter_map(|l| Language::from_name(l)).any(Language::is_data);
    let code = languages.iter().filter_map(|l| Language::from_name(l)).any(|l| !l.is_data());
    if data && code {
        report.error(
            &format!("{path}.language"),
            "JSON, YAML and TOML cannot share a rule with code languages",
            Some("their checks read key paths, not syntax; make one rule for the data files and one for the code".into()),
        );
    }
    if data {
        if let Some(value) = rule.get("forbid_property") {
            path_patterns(value, &format!("{path}.forbid_property"), report);
        }
        if let Some(Value::Mapping(pv)) = rule.get("property_value")
            && let Some(names) = pv.get("names")
        {
            path_patterns(names, &format!("{path}.property_value.names"), report);
        }
    }
    if let Some(value) = rule.get("require_property") {
        *checks += 1;
        let at = format!("{path}.require_property");
        path_patterns(value, &at, report);
        for (i, pattern) in string_or_list(value, &at, &mut Report::default()).iter().enumerate() {
            if PathPattern::parse(pattern).ok().is_some_and(|p| p.split_last().is_none()) {
                report.error(
                    &format!("{at}[{i}]"),
                    format!("`{pattern}` ends in a wildcard, so it names no key that must exist"),
                    Some("end the path with a key name: `jobs.*.timeout-minutes`".into()),
                );
            }
        }
    }
    if let Some(value) = rule.get("same_keys_as") {
        *checks += 1;
        if !matches!(value, Value::String(s) if !s.trim().is_empty()) {
            report.error(&format!("{path}.same_keys_as"), format!("expected a file path, found {}", describe(value)), Some("e.g. `same_keys_as: src/locales/en.json`".into()));
        }
    }
    if let Some(value) = rule.get("same_placeholders") {
        if !matches!(value, Value::Bool(_)) {
            report.error(&format!("{path}.same_placeholders"), format!("expected true or false, found {}", describe(value)), None);
        }
        if rule.get("same_keys_as").is_none() {
            report.error(&format!("{path}.same_placeholders"), "same_placeholders needs same_keys_as", Some("add `same_keys_as: <reference file>`".into()));
        }
    }
    if let Some(value) = rule.get("literal_argument") {
        *checks += 1;
        validate_literal_argument(value, &format!("{path}.literal_argument"), report);
    }
}
