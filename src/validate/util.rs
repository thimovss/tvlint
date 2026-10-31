//! Small helpers shared by the validators: reading lists and mappings, and suggesting close names.

use globset::GlobBuilder;
use serde_yaml_ng::{Mapping, Value};

use super::report::Report;

/// A string or a list of strings.
pub(super) fn string_or_list(value: &Value, path: &str, report: &mut Report) -> Vec<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => vec![s.clone()],
        Value::Sequence(_) => string_list(value, path, report),
        other => {
            report.error(path, format!("expected a name or a list of names, found {}", describe(other)), None);
            Vec::new()
        }
    }
}

pub(super) fn glob_list(value: &Value, path: &str, report: &mut Report) {
    let patterns = string_list(value, path, report);
    for (i, pattern) in patterns.iter().enumerate() {
        if let Err(err) = GlobBuilder::new(pattern).literal_separator(true).build() {
            report.error(&format!("{path}[{i}]"), format!("invalid glob `{pattern}`: {}", err.kind()), None);
        }
    }
}

/// A non-empty list of non-empty strings. Reports problems and returns the usable entries.
pub(super) fn string_list(value: &Value, path: &str, report: &mut Report) -> Vec<String> {
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

pub(super) fn expect_mapping<'a>(value: &'a Value, path: &str, expected: &str, report: &mut Report) -> Option<&'a Mapping> {
    let map = value.as_mapping();
    if map.is_none() {
        report.error(path, format!("expected {expected}, found {}", describe(value)), None);
    }
    map
}

pub(super) fn check_keys(map: &Mapping, path: &str, allowed: &[(&str, &str)], report: &mut Report) {
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

pub(super) fn with_suggestion<'a>(word: &str, candidates: impl Iterator<Item = &'a str>, otherwise: &str) -> String {
    match closest(word, candidates) {
        Some(near) => format!("did you mean `{near}`? ({otherwise})"),
        None => otherwise.to_string(),
    }
}

pub(super) fn closest<'a>(word: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (word.len() / 3).max(2);
    candidates
        .map(|c| (edit_distance(word, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

pub(super) fn edit_distance(a: &str, b: &str) -> usize {
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

pub(super) fn describe(value: &Value) -> &'static str {
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
