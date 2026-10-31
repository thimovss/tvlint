use std::path::Path;

use anyhow::{Context, Result, bail};

/// An empty but valid config. The commented rule shows the shape; nothing is enforced until the user writes one.
const TEMPLATE: &str = "\
# tvlint.yaml: the rules of this project, checked deterministically.
#
#   tvl validate        is this file correct?
#   tvl check           does the code follow it?
#   tvl explain <rule>  why does a rule exist, and how do I comply?
#
# A rule says what is not allowed and why. Every rule carries a `rationale` and a `fix`, which
# tvl prints with each violation so a developer or an AI agent knows how to comply.
# `scopes` say which files a rule applies to. For example:
#
# rules:
#   no-print-in-src:
#     language: python
#     forbid_call: [print]
#     rationale: Output goes through the logger so it can be filtered and collected.
#     fix: Use logging.getLogger(__name__).info(...).
#
# scopes:
#   - rules: [no-print-in-src]
#     include: [\"src/**\"]

rules: {}

scopes: []
";

/// Writes an empty config at `config_path`. Never overwrites an existing file unless `force` is set.
pub fn run(config_path: &Path, force: bool) -> Result<()> {
    if config_path.exists() && !force {
        bail!("{} already exists; pass --force to overwrite it", config_path.display());
    }
    std::fs::write(config_path, TEMPLATE).with_context(|| format!("cannot write {}", config_path.display()))?;
    println!("Created {}, with no rules yet.", config_path.display());
    println!("Add rules under `rules:` and the files they apply to under `scopes:`, then run tvl validate.");
    println!("`tvl schema` prints the JSON Schema of the file, for editors and AI agents.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn template_is_a_loadable_config_with_no_rules() {
        let config = Config::parse(TEMPLATE).unwrap();
        assert!(config.rules.is_empty());
        assert!(config.scopes.is_empty());
    }

    #[test]
    fn the_commented_example_is_a_valid_rule() {
        let example: String = TEMPLATE
            .lines()
            .skip_while(|l| *l != "# rules:")
            .take_while(|l| l.starts_with('#'))
            .map(|l| l.strip_prefix("# ").or_else(|| l.strip_prefix('#')).unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n");
        let config = Config::parse(&example).unwrap();
        assert!(config.rules.contains_key("no-print-in-src"));
    }
}
