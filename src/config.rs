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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub language: Language,
    #[serde(default)]
    pub forbid_raise: Vec<String>,
    #[serde(default)]
    pub forbid_call: Vec<String>,
    pub forbid_catch: Option<ClassSelector>,
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
            if rule.forbid_raise.is_empty() && rule.forbid_call.is_empty() && rule.forbid_catch.is_none() {
                bail!("rule `{id}` has no checks (expected forbid_raise, forbid_call or forbid_catch)");
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
