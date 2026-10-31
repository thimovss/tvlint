//! A rule as written in `tvlint.yaml`, and the settings of its checks.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Deserializer};

use super::language::Language;

/// Accepts `names: queryKey` as well as `names: [queryKey, other]`.
pub(super) fn string_or_list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(item) => vec![item],
        OneOrMany::Many(items) => items,
    })
}

/// Accepts `language: python` as well as `language: [javascript, typescript]`.
pub(super) fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Language>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(Language),
        Many(Vec<Language>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(language) => vec![language],
        OneOrMany::Many(languages) => languages,
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Which files the rule applies to. Required for code checks; empty means every
    /// file, which only makes sense for file-name rules.
    #[serde(default, deserialize_with = "one_or_many")]
    pub language: Vec<Language>,
    #[serde(default)]
    pub forbid_raise: Vec<String>,
    #[serde(default)]
    pub forbid_call: Vec<String>,
    pub forbid_catch: Option<ClassSelector>,
    /// Module specifiers that may not be imported. Relative and aliased imports are matched
    /// by their project path, anything else by the specifier as written.
    #[serde(default)]
    pub forbid_import: Vec<String>,
    /// Imports that are fine even though they match `forbid_import`.
    #[serde(default)]
    pub except_import: Vec<String>,
    /// JSX element names that may not be used, e.g. `button`.
    #[serde(default)]
    pub forbid_element: Vec<String>,
    /// Object-literal property names that may not appear.
    #[serde(default)]
    pub forbid_property: Vec<String>,
    /// Constraints on the value of object-literal properties.
    pub property_value: Option<PropertyValue>,
    /// Only count `forbid_call`, `forbid_raise` and `forbid_element` hits that sit inside
    /// an object-literal property with one of these names, e.g. `onError`.
    #[serde(default)]
    pub within_property: Vec<String>,
    /// Key paths that must exist, e.g. `jobs.*.timeout-minutes`: every match of the parent path must have the key.
    #[serde(default)]
    pub require_property: Vec<String>,
    /// A file (relative to the config) whose key paths every matched data file must have too.
    pub same_keys_as: Option<String>,
    /// Also require the `{{placeholders}}` of each shared message to be the same as in `same_keys_as`.
    #[serde(default)]
    pub same_placeholders: bool,
    /// String literals passed to calls must be listed in a data file.
    pub literal_argument: Option<LiteralArgument>,
    /// Calls that must appear: at least one of these in each file, or in each function that `within_function`
    /// selects.
    #[serde(default)]
    pub require_call: Vec<String>,
    /// Methods that every selected class must declare itself.
    #[serde(default)]
    pub require_method: Vec<String>,
    /// Fields (annotated or assigned names in the class body) that every selected class must declare itself.
    #[serde(default)]
    pub require_field: Vec<String>,
    /// Fields that no selected class may declare.
    #[serde(default)]
    pub forbid_field: Vec<String>,
    /// Which classes `require_method`, `require_field` and `forbid_field` look at. Default: every class.
    pub in_class: Option<ClassFilter>,
    /// Only count `forbid_call` and `forbid_raise` hits inside a function with one of these names, and
    /// make `require_call` apply to each of those functions instead of to the file.
    #[serde(default)]
    pub within_function: Vec<String>,
    /// Functions that `within_function` leaves out, e.g. `_*` for private ones.
    #[serde(default)]
    pub except_function: Vec<String>,
    /// Assignment targets, as globs on the target as written (`aggregate.*`), that may not be assigned.
    #[serde(default)]
    pub forbid_assignment: Vec<String>,
    /// Like `forbid_assignment`, for `+=` and the like only.
    #[serde(default)]
    pub forbid_augmented_assignment: Vec<String>,
    /// String literals that may not be compared or assigned to constants.
    pub forbid_literal: Option<ForbidLiteral>,
    /// Calls that must pass these keyword arguments.
    pub require_argument: Option<ArgumentCheck>,
    /// Calls that must not pass these keyword arguments.
    pub forbid_argument: Option<ArgumentCheck>,
    /// Constraints on the keys of a dict literal passed to a call.
    pub argument_keys: Option<ArgumentKeys>,
    /// Only these exceptions may be raised.
    pub allow_raise: Option<AllowRaise>,
    /// String literals assigned to names must be listed in a data file and/or have a shape.
    pub literal_assignment: Option<LiteralArgument>,
    /// Every matched file's name must match one of these globs.
    #[serde(default)]
    pub require_filename: Vec<String>,
    /// Files with more lines than this are reported.
    pub max_lines: Option<usize>,
    pub rationale: String,
    pub fix: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyValue {
    #[serde(deserialize_with = "string_or_list")]
    pub names: Vec<String>,
    /// Only look at objects passed directly as an argument to one of these calls.
    #[serde(default, deserialize_with = "string_or_list")]
    pub in_call: Vec<String>,
    /// The value must not be a string literal.
    #[serde(default)]
    pub forbid_literal: bool,
    /// An array-literal value must start with this element (matched as a glob on its source text).
    pub array_starts_with: Option<String>,
    /// Data files: the value must not match any of these globs, e.g. `^*` or `*:latest`.
    #[serde(default, deserialize_with = "string_or_list")]
    pub forbid_value: Vec<String>,
    /// Data files: the value must match one of these globs.
    #[serde(default, deserialize_with = "string_or_list")]
    pub require_value: Vec<String>,
}

/// String literals that must be listed in a data file and/or have a certain shape. `literal_argument` watches
/// the arguments of calls (`calls`, `position`); `literal_assignment` watches what is assigned to names (`names`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteralArgument {
    /// Calls whose argument is checked, as patterns like `forbid_call`.
    #[serde(default, deserialize_with = "string_or_list")]
    pub calls: Vec<String>,
    /// Assignment targets whose value is checked, as globs on the target as written (`event_type`, `self.*`).
    #[serde(default, deserialize_with = "string_or_list")]
    pub names: Vec<String>,
    /// Which argument, counting from 0.
    #[serde(default)]
    pub position: usize,
    /// The data file the literals must be listed in.
    pub registry: Option<Registry>,
    /// Also report registry entries that nothing in scope uses.
    #[serde(default)]
    pub report_unused: bool,
    /// Every literal must match one of these globs, e.g. `[a-z]*_failed.[a-z]*`.
    #[serde(default, deserialize_with = "string_or_list")]
    pub pattern: Vec<String>,
    /// A watched argument or value that is not a plain string literal (a variable, an f-string) is reported.
    #[serde(default)]
    pub require_literal: bool,
    /// What is looked up in the registry: the literal itself, or the part before its first dot.
    #[serde(default)]
    pub lookup: Lookup,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lookup {
    #[default]
    Value,
    Prefix,
}

impl Lookup {
    /// The text looked up in the registry for a literal.
    pub fn key(self, literal: &str) -> &str {
        match self {
            Lookup::Value => literal,
            Lookup::Prefix => literal.split('.').next().unwrap_or(literal),
        }
    }
}

/// What a call must (`require_argument`) or must not (`forbid_argument`) pass by keyword.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentCheck {
    /// Calls the check applies to, as patterns like `forbid_call`.
    #[serde(deserialize_with = "string_or_list")]
    pub calls: Vec<String>,
    /// Keyword argument names.
    #[serde(deserialize_with = "string_or_list")]
    pub names: Vec<String>,
}

/// Constraints on the keys of a dict or object literal passed to a call.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentKeys {
    #[serde(deserialize_with = "string_or_list")]
    pub calls: Vec<String>,
    /// Which positional argument, counting from 0. Ignored when `keyword` is given.
    #[serde(default)]
    pub position: usize,
    /// Look at the keyword argument with this name instead of a positional one.
    pub keyword: Option<String>,
    /// Key globs that may not appear, e.g. `*_id`.
    #[serde(default, deserialize_with = "string_or_list")]
    pub forbid_keys: Vec<String>,
    /// Every key must match one of these globs.
    #[serde(default, deserialize_with = "string_or_list")]
    pub allow_keys: Vec<String>,
}

/// String literals that may not be compared or assigned.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForbidLiteral {
    /// A string literal on either side of `==`, `!=`, `===` or `!==`.
    #[serde(default)]
    pub compared: bool,
    /// A string literal assigned to a name matching one of these globs, e.g. `[A-Z]*` for constants.
    #[serde(default, deserialize_with = "string_or_list")]
    pub assigned_to: Vec<String>,
}

/// Only these exceptions may be raised.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowRaise {
    /// Exception name globs, matched on the full dotted name or its last part.
    #[serde(default, deserialize_with = "string_or_list")]
    pub names: Vec<String>,
    /// This class and everything that inherits from it.
    pub subclass_of: Option<String>,
}

/// What a registry entry is called: the matched key, the matched scalar, or the scalar's full path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Use {
    Key,
    Value,
    Path,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// The data file listing the allowed values, relative to the config.
    pub file: String,
    /// Which nodes of the file are entries.
    pub path: String,
    /// What an entry is called: its `key`, its `value` (default) or its full `path`.
    #[serde(default = "default_use", rename = "use")]
    pub use_: Use,
}

pub(super) fn default_use() -> Use {
    Use::Value
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassSelector {
    pub subclass_of: String,
}

/// Which classes a class check looks at: the ones that match every key given.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassFilter {
    /// Classes that inherit from this one, directly or not. The class itself is not included.
    pub subclass_of: Option<String>,
    /// Class name globs.
    #[serde(default, deserialize_with = "string_or_list")]
    pub names: Vec<String>,
}

impl ClassFilter {
    /// `subclasses of X, named *Event`.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(base) = &self.subclass_of {
            parts.push(format!("subclasses of {base}"));
        }
        if !self.names.is_empty() {
            parts.push(format!("named {}", self.names.join("/")));
        }
        parts.join(", ")
    }
}

impl Rule {
    /// Whether the rule has a check that `within_function` can narrow to some functions.
    pub fn narrowable_by_function(&self) -> bool {
        !self.forbid_call.is_empty()
            || !self.forbid_raise.is_empty()
            || !self.require_call.is_empty()
            || !self.forbid_assignment.is_empty()
            || !self.forbid_augmented_assignment.is_empty()
            || self.forbid_literal.is_some()
            || self.require_argument.is_some()
            || self.forbid_argument.is_some()
            || self.argument_keys.is_some()
            || self.allow_raise.is_some()
    }

    /// The literal check, from `literal_argument` or `literal_assignment`.
    pub fn literal_check(&self) -> Option<&LiteralArgument> {
        self.literal_argument.as_ref().or(self.literal_assignment.as_ref())
    }

    pub fn has_code_checks(&self) -> bool {
        !self.code_keys().is_empty()
    }

    /// The code checks this rule uses, by rule key.
    pub fn code_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        for (key, present) in [
            ("forbid_raise", !self.forbid_raise.is_empty()),
            ("forbid_call", !self.forbid_call.is_empty()),
            ("forbid_catch", self.forbid_catch.is_some()),
            ("forbid_import", !self.forbid_import.is_empty()),
            ("except_import", !self.except_import.is_empty()),
            ("forbid_element", !self.forbid_element.is_empty()),
            ("forbid_property", !self.forbid_property.is_empty()),
            ("property_value", self.property_value.is_some()),
            ("within_property", !self.within_property.is_empty()),
            ("require_property", !self.require_property.is_empty()),
            ("same_keys_as", self.same_keys_as.is_some()),
            ("same_placeholders", self.same_placeholders),
            ("literal_argument", self.literal_argument.is_some()),
            ("max_lines", self.max_lines.is_some()),
            ("forbid_assignment", !self.forbid_assignment.is_empty()),
            ("forbid_augmented_assignment", !self.forbid_augmented_assignment.is_empty()),
            ("forbid_literal", self.forbid_literal.is_some()),
            ("require_argument", self.require_argument.is_some()),
            ("forbid_argument", self.forbid_argument.is_some()),
            ("argument_keys", self.argument_keys.is_some()),
            ("allow_raise", self.allow_raise.is_some()),
            ("literal_assignment", self.literal_assignment.is_some()),
            ("require_call", !self.require_call.is_empty()),
            ("require_method", !self.require_method.is_empty()),
            ("require_field", !self.require_field.is_empty()),
            ("forbid_field", !self.forbid_field.is_empty()),
            ("in_class", self.in_class.is_some()),
            ("within_function", !self.within_function.is_empty()),
            ("except_function", !self.except_function.is_empty()),
        ] {
            if present {
                keys.push(key);
            }
        }
        keys
    }

    /// The first check that one of the rule's languages cannot express.
    pub fn unsupported(&self) -> Option<(&'static str, Language)> {
        self.code_keys()
            .into_iter()
            .find_map(|key| self.language.iter().find(|l| !l.supports(key)).map(|l| (key, *l)))
    }

    /// One line per check, for `explain` and `validate`.
    pub fn describe_checks(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.forbid_raise.is_empty() {
            out.push(format!("raising {}", self.forbid_raise.join(", ")));
        }
        if !self.forbid_call.is_empty() {
            out.push(format!("calling {}", self.forbid_call.join(", ")));
        }
        if let Some(catch) = &self.forbid_catch {
            out.push(format!("catching {} or its subclasses", catch.subclass_of));
        }
        if !self.forbid_import.is_empty() {
            let except = if self.except_import.is_empty() {
                String::new()
            } else {
                format!(" (except {})", self.except_import.join(", "))
            };
            out.push(format!("importing {}{except}", self.forbid_import.join(", ")));
        }
        if !self.forbid_element.is_empty() {
            let tags: Vec<String> = self.forbid_element.iter().map(|e| format!("<{e}>")).collect();
            out.push(format!("using {}", tags.join(", ")));
        }
        if !self.forbid_property.is_empty() {
            out.push(format!("object properties named {}", self.forbid_property.join(", ")));
        }
        if let Some(pv) = &self.property_value {
            let mut parts = Vec::new();
            if !pv.forbid_value.is_empty() {
                parts.push(format!("the value {}", pv.forbid_value.join(", ")));
            }
            if !pv.require_value.is_empty() {
                parts.push(format!("any value but {}", pv.require_value.join(", ")));
            }
            if pv.forbid_literal {
                parts.push("string literals".to_string());
            }
            if let Some(start) = &pv.array_starts_with {
                parts.push(format!("arrays not starting with {start}"));
            }
            let context = if pv.in_call.is_empty() { String::new() } else { format!(" in {}(...)", pv.in_call.join("/")) };
            out.push(format!("{} as the value of {}{context}", parts.join(" or "), pv.names.join(", ")));
        }
        if !self.require_property.is_empty() {
            out.push(format!("files without the key {}", self.require_property.join(", ")));
        }
        if let Some(reference) = &self.same_keys_as {
            let what = if self.same_placeholders { "keys and {{placeholders}}" } else { "keys" };
            out.push(format!("files whose {what} differ from {reference}"));
        }
        if let Some(arg) = self.literal_check() {
            let what = if self.literal_assignment.is_some() {
                format!("values assigned to {}", arg.names.join("/"))
            } else {
                format!("argument {} of {}", arg.position, arg.calls.join("/"))
            };
            let mut problems = Vec::new();
            if arg.require_literal {
                problems.push("that is not a plain string literal".to_string());
            }
            if !arg.pattern.is_empty() {
                problems.push(format!("that does not match {}", arg.pattern.join(", ")));
            }
            if let Some(registry) = &arg.registry {
                let of = if arg.lookup == Lookup::Prefix { " (by the part before the dot)" } else { "" };
                problems.push(format!("that is not in {}{of}", registry.file));
            }
            let unused = if arg.report_unused { ", and entries nothing uses" } else { "" };
            out.push(format!("{what} {}{unused}", problems.join(" or ")));
        }
        if !self.forbid_assignment.is_empty() {
            out.push(format!("assigning to {}", self.forbid_assignment.join(", ")));
        }
        if !self.forbid_augmented_assignment.is_empty() {
            out.push(format!("augmented assignment (`+=`) to {}", self.forbid_augmented_assignment.join(", ")));
        }
        if let Some(lit) = &self.forbid_literal {
            if lit.compared {
                out.push("comparing with a string literal".to_string());
            }
            if !lit.assigned_to.is_empty() {
                out.push(format!("assigning a string literal to {}", lit.assigned_to.join(", ")));
            }
        }
        if let Some(check) = &self.require_argument {
            out.push(format!("calls to {} without the keyword {}", check.calls.join("/"), check.names.join(", ")));
        }
        if let Some(check) = &self.forbid_argument {
            out.push(format!("calls to {} that pass the keyword {}", check.calls.join("/"), check.names.join(", ")));
        }
        if let Some(keys) = &self.argument_keys {
            let mut parts = Vec::new();
            if !keys.forbid_keys.is_empty() {
                parts.push(format!("the keys {}", keys.forbid_keys.join(", ")));
            }
            if !keys.allow_keys.is_empty() {
                parts.push(format!("keys other than {}", keys.allow_keys.join(", ")));
            }
            out.push(format!("{} in the dict passed to {}", parts.join(" or "), keys.calls.join("/")));
        }
        if let Some(allow) = &self.allow_raise {
            let mut parts = allow.names.clone();
            parts.extend(allow.subclass_of.iter().map(|c| format!("{c} and its subclasses")));
            out.push(format!("raising anything but {}", parts.join(", ")));
        }
        if !self.within_property.is_empty() {
            out.push(format!("(only inside a property named {})", self.within_property.join(", ")));
        }
        if !self.require_call.is_empty() {
            let scope = if self.within_function.is_empty() { "files" } else { "functions" };
            out.push(format!("{scope} that never call {}", self.require_call.join(", ")));
        }
        let classes = self.in_class.as_ref().map_or_else(String::new, |c| format!(" (classes: {})", c.describe()));
        if !self.require_method.is_empty() {
            out.push(format!("classes without the method {}{classes}", self.require_method.join(", ")));
        }
        if !self.require_field.is_empty() {
            out.push(format!("classes without the field {}{classes}", self.require_field.join(", ")));
        }
        if !self.forbid_field.is_empty() {
            out.push(format!("fields named {}{classes}", self.forbid_field.join(", ")));
        }
        if !self.within_function.is_empty() {
            let except = if self.except_function.is_empty() { String::new() } else { format!(" except {}", self.except_function.join(", ")) };
            out.push(format!("(only inside functions named {}{except})", self.within_function.join(", ")));
        }
        if let Some(max) = self.max_lines {
            out.push(format!("files longer than {max} lines"));
        }
        if !self.require_filename.is_empty() {
            out.push(format!("file names other than {}", self.require_filename.join(", ")));
        }
        out
    }

    /// The highest `{n}` placeholder written in `forbid_import` or `except_import`, 0 if there is none.
    pub fn max_placeholder(&self) -> usize {
        self.forbid_import.iter().chain(&self.except_import).flat_map(|p| super::captures::placeholders(p)).max().unwrap_or(0)
    }

    pub fn applies_to(&self, language: Language) -> bool {
        self.language.is_empty() || self.language.contains(&language)
    }

    pub fn applies_to_path(&self, path: &str) -> bool {
        self.language.is_empty()
            || Language::from_path(Path::new(path)).is_some_and(|language| self.language.contains(&language))
    }

    /// `python`, `javascript/typescript`, or `any language`.
    pub fn language_label(&self) -> String {
        if self.language.is_empty() {
            "any language".to_string()
        } else {
            self.language.iter().map(|l| l.name()).collect::<Vec<_>>().join("/")
        }
    }
}
