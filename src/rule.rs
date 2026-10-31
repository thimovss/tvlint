use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Rule {
    Dependency(DependencyRule),
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyRule {
    pub id: String,
    pub description: String,
    pub severity: Severity,
    pub from: String,
    #[serde(default)]
    pub except_from: Option<String>,
    pub forbid_import: Vec<String>,
    pub rationale: String,
    pub fix: String,
}

impl Rule {
    pub fn from_yaml(source: &str) -> Result<Rule, serde_yaml_ng::Error> {
        serde_yaml_ng::from_str(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
id: no-direct-db-access
description: Database access must go through the repository layer.
severity: error
type: dependency
from: "src/**"
except_from: "src/repositories/**"
forbid_import: ["src/db/**", "pg", "knex"]
rationale: Direct queries bypass auditing and caching.
fix: Use the matching repository in src/repositories/.
"#;

    #[test]
    fn parses_example_rule() {
        let Rule::Dependency(rule) = Rule::from_yaml(EXAMPLE).unwrap();
        assert_eq!(rule.id, "no-direct-db-access");
        assert_eq!(rule.severity, Severity::Error);
        assert_eq!(rule.forbid_import, ["src/db/**", "pg", "knex"]);
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = format!("{EXAMPLE}bogus: 1\n");
        assert!(Rule::from_yaml(&yaml).is_err());
    }

    #[test]
    fn rejects_missing_rationale() {
        let yaml = EXAMPLE.replace("rationale: Direct queries bypass auditing and caching.\n", "");
        assert!(Rule::from_yaml(&yaml).is_err());
    }
}
