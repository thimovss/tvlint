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

pub(super) fn validate_literal_argument(value: &Value, path: &str, assignment: bool, report: &mut Report) {
    const KEYS: &[(&str, &str)] = &[
        ("calls", "literal_argument; call pattern or list of patterns, like forbid_call"),
        ("names", "literal_assignment; assignment target glob or list of globs, e.g. `event_type`"),
        ("position", "literal_argument; which argument to check, counting from 0 (default 0)"),
        ("registry", "`file`, `path` and optionally `use` (key, value or path) of the data file listing the allowed values"),
        ("report_unused", "optional; true also reports registry entries that nothing uses"),
        ("pattern", "optional; glob or list of globs every literal must match, e.g. `[a-z]*_failed.[a-z]*`"),
        ("require_literal", "optional; true reports a watched value that is not a plain string literal"),
        ("lookup", "optional; `value` (default) or `prefix`: look up the literal itself or the part before its first dot"),
    ];
    let Some(map) = expect_mapping(value, path, "a mapping with `calls` (or `names`) and a check", report) else { return };
    check_keys(map, path, KEYS, report);
    let (watched, hint) = if assignment { ("names", "add `names: [event_type]`") } else { ("calls", "add `calls: [\"*Exception\"]`") };
    match map.get(watched) {
        None => report.error(path, format!("missing required key `{watched}`"), Some(hint.into())),
        Some(v) => {
            for (i, pattern) in string_or_list(v, &format!("{path}.{watched}"), report).iter().enumerate() {
                if let Err(err) = Glob::new(pattern) {
                    report.error(&format!("{path}.{watched}[{i}]"), format!("invalid pattern `{pattern}`: {}", err.kind()), None);
                }
            }
        }
    }
    let other = if assignment { "calls" } else { "names" };
    if map.get(other).is_some() {
        report.error(&format!("{path}.{other}"), format!("`{other}` belongs to {}", if assignment { "literal_argument" } else { "literal_assignment" }), None);
    }
    match map.get("position") {
        None | Some(Value::Number(_)) => {}
        Some(other) => report.error(&format!("{path}.position"), format!("expected an argument number, found {}", describe(other)), None),
    }
    for key in ["report_unused", "require_literal"] {
        match map.get(key) {
            None | Some(Value::Bool(_)) => {}
            Some(other) => report.error(&format!("{path}.{key}"), format!("expected true or false, found {}", describe(other)), None),
        }
    }
    if let Some(v) = map.get("pattern") {
        for (i, pattern) in string_or_list(v, &format!("{path}.pattern"), report).iter().enumerate() {
            if let Err(err) = Glob::new(pattern) {
                report.error(&format!("{path}.pattern[{i}]"), format!("invalid glob `{pattern}`: {}", err.kind()), None);
            }
        }
    }
    match map.get("lookup") {
        None => {}
        Some(Value::String(s)) if ["value", "prefix"].contains(&s.as_str()) => {}
        Some(other) => report.error(&format!("{path}.lookup"), format!("expected value or prefix, found {}", other.as_str().map_or_else(|| describe(other).to_string(), |s| format!("`{s}`"))), None),
    }
    let registry_path = format!("{path}.registry");
    match map.get("registry") {
        None => {
            let shaped = map.get("pattern").is_some() || map.get("require_literal").is_some_and(|v| v.as_bool() == Some(true));
            if !shaped {
                report.error(path, "the literal check has nothing to check", Some("add `registry: { file: codes.json, path: \"codes.*\", use: key }`, a `pattern`, or `require_literal: true`".into()));
            }
            for key in ["report_unused", "lookup"] {
                if map.get(key).is_some() {
                    report.error(&format!("{path}.{key}"), format!("{key} needs a registry"), None);
                }
            }
        }
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
    const NARROWED: &[&str] = &[
        "forbid_call",
        "forbid_raise",
        "require_call",
        "forbid_assignment",
        "forbid_augmented_assignment",
        "forbid_literal",
        "require_argument",
        "forbid_argument",
        "argument_keys",
        "allow_raise",
    ];
    if rule.get("within_function").is_some() && NARROWED.iter().all(|k| rule.get(*k).is_none()) {
        report.error(
            &format!("{path}.within_function"),
            "within_function has nothing to narrow",
            Some("it applies to forbid_call, forbid_raise, require_call, assignment, literal, argument and allow_raise checks".into()),
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

/// Keys about single statements and expressions: assignments, comparisons, call arguments and raises.
pub(super) fn validate_shape_keys(rule: &Mapping, path: &str, checks: &mut usize, report: &mut Report) {
    let globs = |value: &Value, at: &str, report: &mut Report| {
        for (i, pattern) in string_or_list(value, at, report).iter().enumerate() {
            if let Err(err) = Glob::new(pattern) {
                report.error(&format!("{at}[{i}]"), format!("invalid glob `{pattern}`: {}", err.kind()), None);
            }
        }
    };
    for key in ["forbid_assignment", "forbid_augmented_assignment"] {
        if let Some(value) = rule.get(key) {
            *checks += 1;
            globs(value, &format!("{path}.{key}"), report);
        }
    }
    if let Some(value) = rule.get("forbid_literal") {
        *checks += 1;
        let at = format!("{path}.forbid_literal");
        if let Some(map) = expect_mapping(value, &at, "a mapping like `{ compared: true }`", report) {
            check_keys(map, &at, &[("compared", "true: a string literal on either side of ==, !=, === or !=="), ("assigned_to", "target glob or list of globs a string literal may not be assigned to, e.g. `[A-Z]*`")], report);
            match map.get("compared") {
                None | Some(Value::Bool(_)) => {}
                Some(other) => report.error(&format!("{at}.compared"), format!("expected true or false, found {}", describe(other)), None),
            }
            if let Some(v) = map.get("assigned_to") {
                globs(v, &format!("{at}.assigned_to"), report);
            }
            if map.get("assigned_to").is_none() && map.get("compared").is_none_or(|v| v.as_bool() != Some(true)) {
                report.error(&at, "forbid_literal has nothing to forbid", Some("add `compared: true` or `assigned_to: [\"[A-Z]*\"]`".into()));
            }
        }
    }
    for key in ["require_argument", "forbid_argument"] {
        let Some(value) = rule.get(key) else { continue };
        *checks += 1;
        let at = format!("{path}.{key}");
        let Some(map) = expect_mapping(value, &at, "a mapping with `calls` and `names`", report) else { continue };
        check_keys(map, &at, &[("calls", "required; call pattern or list of patterns, like forbid_call"), ("names", "required; keyword argument name or list of names")], report);
        for field in ["calls", "names"] {
            match map.get(field) {
                None => report.error(&at, format!("missing required key `{field}`"), None),
                Some(v) if field == "calls" => globs(v, &format!("{at}.calls"), report),
                Some(v) => {
                    string_or_list(v, &format!("{at}.names"), report);
                }
            }
        }
    }
    if let Some(value) = rule.get("argument_keys") {
        *checks += 1;
        let at = format!("{path}.argument_keys");
        if let Some(map) = expect_mapping(value, &at, "a mapping with `calls` and `forbid_keys` or `allow_keys`", report) {
            check_keys(
                map,
                &at,
                &[
                    ("calls", "required; call pattern or list of patterns"),
                    ("position", "which positional argument holds the dict, counting from 0 (default 0)"),
                    ("keyword", "look at the keyword argument with this name instead"),
                    ("forbid_keys", "key glob or list of globs that may not appear"),
                    ("allow_keys", "key glob or list of globs; every key must match one"),
                ],
                report,
            );
            match map.get("calls") {
                None => report.error(&at, "missing required key `calls`", None),
                Some(v) => globs(v, &format!("{at}.calls"), report),
            }
            for field in ["forbid_keys", "allow_keys"] {
                if let Some(v) = map.get(field) {
                    globs(v, &format!("{at}.{field}"), report);
                }
            }
            if map.get("forbid_keys").is_none() && map.get("allow_keys").is_none() {
                report.error(&at, "argument_keys has nothing to check", Some("add `forbid_keys` or `allow_keys`".into()));
            }
        }
    }
    if let Some(value) = rule.get("allow_raise") {
        *checks += 1;
        let at = format!("{path}.allow_raise");
        if let Some(map) = expect_mapping(value, &at, "a mapping like `{ subclass_of: LocusException }`", report) {
            check_keys(map, &at, &[("names", "exception name glob or list of globs"), ("subclass_of", "a class name: it and everything inheriting from it")], report);
            if let Some(v) = map.get("names") {
                globs(v, &format!("{at}.names"), report);
            }
            if map.get("names").is_none() && map.get("subclass_of").is_none() {
                report.error(&at, "allow_raise allows nothing", Some("add `names` or `subclass_of`".into()));
            }
        }
    }

}
