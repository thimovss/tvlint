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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteralArgument {
    /// Calls whose argument is checked, as patterns like `forbid_call`.
    #[serde(deserialize_with = "string_or_list")]
    pub calls: Vec<String>,
    /// Which argument, counting from 0.
    #[serde(default)]
    pub position: usize,
    pub registry: Registry,
    /// Also report registry entries that no call in scope uses.
    #[serde(default)]
    pub report_unused: bool,
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

impl Rule {
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
        if let Some(arg) = &self.literal_argument {
            let unused = if arg.report_unused { ", and entries nothing uses" } else { "" };
            out.push(format!(
                "argument {} of {} that is not in {}{unused}",
                arg.position,
                arg.calls.join("/"),
                arg.registry.file
            ));
        }
        if !self.within_property.is_empty() {
            out.push(format!("(only inside a property named {})", self.within_property.join(", ")));
        }
        if !self.require_call.is_empty() {
            let scope = if self.within_function.is_empty() { "files" } else { "functions" };
            out.push(format!("{scope} that never call {}", self.require_call.join(", ")));
        }
        let classes = self.in_class.as_ref().map_or_else(String::new, |c| {
            let mut parts = Vec::new();
            if let Some(base) = &c.subclass_of {
                parts.push(format!("subclasses of {base}"));
            }
            if !c.names.is_empty() {
                parts.push(format!("named {}", c.names.join("/")));
            }
            format!(" (classes: {})", parts.join(", "))
        });
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
