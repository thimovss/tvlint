use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rules: BTreeMap<String, Rule>,
    pub scopes: Vec<Scope>,
    /// Import prefixes that stand for a project path, e.g. `"@/": "src/"` for a tsconfig `paths` entry.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
}

impl Language {
    pub fn name(self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
        }
    }

    pub fn from_path(path: &Path) -> Option<Language> {
        match path.extension()?.to_str()? {
            "py" => Some(Language::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
            _ => None,
        }
    }
}

/// Accepts `names: queryKey` as well as `names: [queryKey, other]`.
fn string_or_list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
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
fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Language>, D::Error> {
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

#[derive(Debug, Deserialize)]
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
    /// Every matched file's name must match one of these globs.
    #[serde(default)]
    pub require_filename: Vec<String>,
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassSelector {
    pub subclass_of: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub rules: Vec<String>,
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl Rule {
    pub fn has_code_checks(&self) -> bool {
        !self.forbid_raise.is_empty()
            || !self.forbid_call.is_empty()
            || self.forbid_catch.is_some()
            || self.uses_ecmascript_checks()
    }

    /// The checks that only exist for JavaScript and TypeScript.
    pub fn ecmascript_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        for (key, present) in [
            ("forbid_import", !self.forbid_import.is_empty()),
            ("except_import", !self.except_import.is_empty()),
            ("forbid_element", !self.forbid_element.is_empty()),
            ("forbid_property", !self.forbid_property.is_empty()),
            ("property_value", self.property_value.is_some()),
            ("within_property", !self.within_property.is_empty()),
        ] {
            if present {
                keys.push(key);
            }
        }
        keys
    }

    pub fn uses_ecmascript_checks(&self) -> bool {
        !self.ecmascript_keys().is_empty()
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
            if pv.forbid_literal {
                parts.push("string literals".to_string());
            }
            if let Some(start) = &pv.array_starts_with {
                parts.push(format!("arrays not starting with {start}"));
            }
            let context = if pv.in_call.is_empty() { String::new() } else { format!(" in {}(...)", pv.in_call.join("/")) };
            out.push(format!("{} as the value of {}{context}", parts.join(" or "), pv.names.join(", ")));
        }
        if !self.within_property.is_empty() {
            out.push(format!("(only inside a property named {})", self.within_property.join(", ")));
        }
        if !self.require_filename.is_empty() {
            out.push(format!("file names other than {}", self.require_filename.join(", ")));
        }
        out
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

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Config::parse(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Config> {
        let config: Config = serde_yaml_ng::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        for (id, rule) in &self.rules {
            if !rule.has_code_checks() && rule.require_filename.is_empty() {
                bail!("rule `{id}` has no checks (expected forbid_raise, forbid_call, forbid_catch, forbid_import, forbid_element, forbid_property, property_value or require_filename)");
            }
            if rule.forbid_catch.is_some() && rule.language.iter().any(|l| *l != Language::Python) {
                bail!("rule `{id}`: forbid_catch is only supported for python, JavaScript and TypeScript catch clauses have no exception type");
            }
            if rule.has_code_checks() && rule.language.is_empty() {
                bail!("rule `{id}` needs `language` because it has code checks (forbid_raise, forbid_call, forbid_catch, forbid_import, ...)");
            }
            if let Some(key) = rule.ecmascript_keys().first()
                && rule.language.contains(&Language::Python)
            {
                bail!("rule `{id}`: {key} is only supported for JavaScript and TypeScript");
            }
            if !rule.except_import.is_empty() && rule.forbid_import.is_empty() {
                bail!("rule `{id}`: except_import needs forbid_import");
            }
            if !rule.within_property.is_empty()
                && rule.forbid_call.is_empty()
                && rule.forbid_raise.is_empty()
                && rule.forbid_element.is_empty()
            {
                bail!("rule `{id}`: within_property needs forbid_call, forbid_raise or forbid_element");
            }
            if let Some(pv) = &rule.property_value
                && !pv.forbid_literal
                && pv.array_starts_with.is_none()
            {
                bail!("rule `{id}`: property_value needs forbid_literal or array_starts_with");
            }
        }
        for scope in &self.scopes {
            for id in &scope.rules {
                if !self.rules.contains_key(id) {
                    bail!("scope references unknown rule `{id}`");
                }
            }
        }
        Ok(())
    }
}
