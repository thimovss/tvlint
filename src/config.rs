use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rules: BTreeMap<String, Rule>,
    pub scopes: Vec<Scope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Python,
}

impl Language {
    pub fn name(self) -> &'static str {
        match self {
            Language::Python => "python",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Which files the rule applies to. Required for code checks; without it a
    /// file-name rule applies to every file.
    pub language: Option<Language>,
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
            if rule.has_code_checks() && rule.language.is_none() {
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
