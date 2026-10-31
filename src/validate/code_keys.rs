//! Validation of the keys that read source code: imports, elements, properties and literal arguments.

use globset::Glob;
use serde_yaml_ng::{Mapping, Value};

use crate::data::PathPattern;

use super::report::Report;
use super::util::{check_keys, describe, expect_mapping, glob_list, string_list, string_or_list};

/// The JS/TS-only keys: forbid_import, except_import, forbid_element, forbid_property, property_value, within_property.
pub(super) fn validate_ecmascript_keys(rule: &Mapping, path: &str, checks: &mut usize, report: &mut Report) {
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

pub(super) fn validate_literal_argument(value: &Value, path: &str, report: &mut Report) {
    const KEYS: &[(&str, &str)] = &[
        ("calls", "required; call pattern or list of patterns, like forbid_call"),
        ("position", "optional; which argument to check, counting from 0 (default 0)"),
        ("registry", "required; `file`, `path` and optionally `use` (key, value or path) of the data file listing the allowed values"),
        ("report_unused", "optional; true also reports registry entries that no call uses"),
    ];
    let Some(map) = expect_mapping(value, path, "a mapping with `calls` and `registry`", report) else { return };
    check_keys(map, path, KEYS, report);
    match map.get("calls") {
        None => report.error(path, "missing required key `calls`", Some("add `calls: [\"*Exception\"]`".into())),
        Some(v) => {
            for (i, pattern) in string_or_list(v, &format!("{path}.calls"), report).iter().enumerate() {
                if let Err(err) = Glob::new(pattern) {
                    report.error(&format!("{path}.calls[{i}]"), format!("invalid pattern `{pattern}`: {}", err.kind()), None);
                }
            }
        }
    }
    match map.get("position") {
        None | Some(Value::Number(_)) => {}
        Some(other) => report.error(&format!("{path}.position"), format!("expected an argument number, found {}", describe(other)), None),
    }
    match map.get("report_unused") {
        None | Some(Value::Bool(_)) => {}
        Some(other) => report.error(&format!("{path}.report_unused"), format!("expected true or false, found {}", describe(other)), None),
    }
    let registry_path = format!("{path}.registry");
    match map.get("registry") {
        None => report.error(path, "missing required key `registry`", Some("add `registry: { file: codes.json, path: \"*\", use: key }`".into())),
        Some(registry) => {
            let Some(reg) = expect_mapping(registry, &registry_path, "a mapping with `file` and `path`", report) else { return };
            check_keys(
                reg,
                &registry_path,
                &[
                    ("file", "required; data file relative to the config"),
                    ("path", "required; key path selecting the entries, e.g. `codes.*`"),
                    ("use", "key, value (default) or path: what an entry is called"),
                ],
                report,
            );
            for key in ["file", "path"] {
                match reg.get(key) {
                    Some(Value::String(s)) if !s.trim().is_empty() => {}
                    None => report.error(&registry_path, format!("missing required key `{key}`"), None),
                    Some(other) => report.error(&format!("{registry_path}.{key}"), format!("expected text, found {}", describe(other)), None),
                }
            }
            if let Some(Value::String(pattern)) = reg.get("path")
                && let Err(err) = PathPattern::parse(pattern)
            {
                report.error(&format!("{registry_path}.path"), format!("invalid key path: {err}"), None);
            }
            match reg.get("use") {
                None => {}
                Some(Value::String(s)) if ["key", "value", "path"].contains(&s.as_str()) => {}
                Some(other) => report.error(
                    &format!("{registry_path}.use"),
                    format!("expected key, value or path, found {}", other.as_str().map_or_else(|| describe(other).to_string(), |s| format!("`{s}`"))),
                    None,
                ),
            }
        }
    }
}

pub(super) fn validate_property_value(value: &Value, path: &str, report: &mut Report) {
    const KEYS: &[(&str, &str)] = &[
        ("names", "required; property name or list of names to look at"),
        ("in_call", "optional; only objects passed directly to one of these calls, e.g. `toast.add`"),
        ("forbid_literal", "true: the value must not be a string literal"),
        ("array_starts_with", "an array-literal value must start with this element, e.g. `accountID`"),
        ("forbid_value", "JSON/YAML/TOML; glob or list of globs the value must not match, e.g. `[\"^*\", latest]`"),
        ("require_value", "JSON/YAML/TOML; glob or list of globs, one of which the value must match"),
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
    for key in ["forbid_value", "require_value"] {
        if let Some(v) = map.get(key) {
            for (i, pattern) in string_or_list(v, &format!("{path}.{key}"), report).iter().enumerate() {
                if let Err(err) = Glob::new(pattern) {
                    report.error(&format!("{path}.{key}[{i}]"), format!("invalid glob `{pattern}`: {}", err.kind()), None);
                }
            }
        }
    }
    let values = map.get("forbid_value").is_some() || map.get("require_value").is_some();
    if !literal && !starts && !values && !report.errors.iter().any(|d| d.path.starts_with(path)) {
        report.error(
            path,
            "the property_value has no constraint",
            Some("add `forbid_literal: true` or `array_starts_with: <element>` (JS/TS), or `forbid_value` / `require_value` (JSON/YAML/TOML)".into()),
        );
    }
}

/// The keys about the shape of code: required calls, class members, and function scoping.
pub(super) fn validate_structure_keys(rule: &Mapping, path: &str, checks: &mut usize, report: &mut Report) {
    if let Some(value) = rule.get("require_call") {
        *checks += 1;
        let call_path = format!("{path}.require_call");
        for (i, pattern) in string_list(value, &call_path, report).iter().enumerate() {
            if let Err(err) = Glob::new(pattern) {
                report.error(&format!("{call_path}[{i}]"), format!("invalid pattern `{pattern}`: {}", err.kind()), None);
            }
        }
    }
    for key in ["require_method", "require_field", "forbid_field", "within_function", "except_function"] {
        let Some(value) = rule.get(key) else { continue };
        if key.starts_with("require_") || key == "forbid_field" {
            *checks += 1;
        }
        let list_path = format!("{path}.{key}");
        for (i, pattern) in string_list(value, &list_path, report).iter().enumerate() {
            if let Err(err) = Glob::new(pattern) {
                report.error(&format!("{list_path}[{i}]"), format!("invalid glob `{pattern}`: {}", err.kind()), None);
            }
        }
    }
    let class_checks = ["require_method", "require_field", "forbid_field"].iter().any(|k| rule.get(*k).is_some());
    if let Some(value) = rule.get("in_class") {
        let class_path = format!("{path}.in_class");
        if let Some(map) = expect_mapping(value, &class_path, "a mapping like `{ subclass_of: DomainEvent }`", report) {
            check_keys(
                map,
                &class_path,
                &[
                    ("subclass_of", "class name; classes inheriting from it, directly or not (not the class itself)"),
                    ("names", "class name glob or list of globs"),
                ],
                report,
            );
            match map.get("subclass_of") {
                None => {}
                Some(Value::String(s)) if !s.trim().is_empty() => {}
                Some(other) => report.error(&format!("{class_path}.subclass_of"), format!("expected a class name, found {}", describe(other)), None),
            }
            if let Some(names) = map.get("names") {
                string_or_list(names, &format!("{class_path}.names"), report);
            }
            if map.get("subclass_of").is_none() && map.get("names").is_none() {
                report.error(&class_path, "in_class selects nothing", Some("give `subclass_of` or `names`, or leave in_class out to check every class".into()));
            }
        }
        if !class_checks {
            report.error(&class_path, "in_class has nothing to apply to", Some("it narrows require_method, require_field and forbid_field".into()));
        }
    }
    if rule.get("within_function").is_some() && ["forbid_call", "forbid_raise", "require_call"].iter().all(|k| rule.get(*k).is_none()) {
        report.error(
            &format!("{path}.within_function"),
            "within_function has nothing to narrow",
            Some("it applies to forbid_call, forbid_raise and require_call".into()),
        );
    }
    if rule.get("except_function").is_some() && rule.get("within_function").is_none() {
        report.error(
            &format!("{path}.except_function"),
            "except_function has nothing to except without within_function",
            Some("add `within_function: [\"*\"]` to start from every function".into()),
        );
    }
}
