//! Validation of one rule: its required keys and which checks its languages can express.

use globset::Glob;
use serde_yaml_ng::{Mapping, Value};

use crate::config::Language;

use super::code_keys::{validate_ecmascript_keys, validate_shape_keys, validate_structure_keys};
use super::data_keys::validate_data_keys;
use super::keys::{LANGUAGES, RULE_KEYS};
use super::report::Report;
use super::util::{check_keys, describe, expect_mapping, string_list, with_suggestion};

pub(super) fn validate_rule(path: &str, value: &Value, report: &mut Report) {
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
        "require_property",
        "same_keys_as",
        "literal_argument",
        "max_lines",
        "require_call",
        "require_method",
        "require_field",
        "forbid_field",
        "literal_assignment",
        "forbid_assignment",
        "forbid_augmented_assignment",
        "forbid_literal",
        "require_argument",
        "forbid_argument",
        "argument_keys",
        "allow_raise",
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
    if let Some(value) = rule.get("max_lines") {
        checks += 1;
        match value.as_u64() {
            Some(n) if n > 0 => {}
            _ => report.error(&format!("{path}.max_lines"), format!("expected a positive whole number, found {}", describe(value)), Some("write the largest number of lines a file may have, e.g. `max_lines: 1000`".into())),
        }
    }
    if let Some(value) = rule.get("forbid_catch") {
        checks += 1;
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
    validate_language_support(rule, path, &languages, report);
    validate_ecmascript_keys(rule, path, &mut checks, report);
    validate_structure_keys(rule, path, &mut checks, report);
    validate_shape_keys(rule, path, &mut checks, report);
    validate_data_keys(rule, path, &languages, &mut checks, report);
    if checks == 0 {
        report.error(
            path,
            "the rule has no checks, so it can never report anything",
            Some("add at least one check: forbid_raise, forbid_call, forbid_catch, forbid_import, forbid_element, forbid_property, property_value, max_lines, require_call, require_method, require_field, forbid_field, forbid_assignment, forbid_literal, require_argument, argument_keys, allow_raise or require_filename".into()),
        );
    }
}

/// Each language can express some checks and not others: say which, and what to use instead.
pub(super) fn validate_language_support(rule: &Mapping, path: &str, languages: &[String], report: &mut Report) {
    const KEYS: &[&str] = &[
        "forbid_raise",
        "forbid_call",
        "forbid_catch",
        "forbid_import",
        "except_import",
        "forbid_element",
        "forbid_property",
        "property_value",
        "within_property",
        "require_property",
        "same_keys_as",
        "same_placeholders",
        "require_method",
        "require_field",
        "forbid_field",
        "in_class",
        "within_function",
        "except_function",
        "forbid_assignment",
        "forbid_augmented_assignment",
        "forbid_literal",
        "literal_assignment",
        "argument_keys",
        "allow_raise",
        "require_argument",
        "forbid_argument",
    ];
    for key in KEYS.iter().filter(|k| rule.get(**k).is_some()) {
        for language in languages.iter().filter_map(|l| Language::from_name(l)) {
            if !language.supports(key) {
                report.error(
                    &format!("{path}.{key}"),
                    format!("{key} is not supported for {}", language.name()),
                    Some(format!("{}; remove {} from `language`, or move this check to a rule of its own", language.unsupported_hint(key), language.name())),
                );
            }
        }
    }
}

/// `language: python` or `language: [javascript, typescript]`. Returns the valid names.
pub(super) fn validate_languages(value: &Value, path: &str, report: &mut Report) -> Vec<String> {
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
