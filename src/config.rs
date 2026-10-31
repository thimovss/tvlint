use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rules: BTreeMap<String, Rule>,
    pub scopes: Vec<Scope>,
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
    /// Every matched file's name must match one of these globs.
    #[serde(default)]
    pub require_filename: Vec<String>,
    pub rationale: String,
    pub fix: String,
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
        !self.forbid_raise.is_empty() || !self.forbid_call.is_empty() || self.forbid_catch.is_some()
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
                bail!("rule `{id}` has no checks (expected forbid_raise, forbid_call, forbid_catch or require_filename)");
            }
            if rule.forbid_catch.is_some() && rule.language.iter().any(|l| *l != Language::Python) {
                bail!("rule `{id}`: forbid_catch is only supported for python, JavaScript and TypeScript catch clauses have no exception type");
            }
            if rule.has_code_checks() && rule.language.is_empty() {
                bail!("rule `{id}` needs `language` because it has forbid_raise, forbid_call or forbid_catch");
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
