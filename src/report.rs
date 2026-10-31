use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::check::Violation;
use crate::config::Rule;

/// Numbers shown at the end of a check, in every output format.
pub struct Summary {
    pub files_checked: usize,
    pub files_with_violations: usize,
    /// Violations hidden by `prlint-ignore-next-line`.
    pub suppressed: usize,
    /// Files skipped by `prlint-ignore-file`.
    pub files_ignored: usize,
}

/// One violation as a record an agent or script can read without parsing text.
fn violation_json(v: &Violation, rule: &Rule) -> Value {
    json!({
        "rule": v.rule_id,
        "file": v.path,
        "line": v.line,
        "column": v.column,
        "message": v.message,
        "rationale": rule.rationale.trim(),
        "fix": rule.fix.trim(),
    })
}

pub fn json(violations: &[Violation], rules: &BTreeMap<String, Rule>, summary: &Summary) -> String {
    let document = json!({
        "violations": violations.iter().map(|v| violation_json(v, &rules[&v.rule_id])).collect::<Vec<_>>(),
        "summary": {
            "violations": violations.len(),
            "files_with_violations": summary.files_with_violations,
            "files_checked": summary.files_checked,
            "suppressed": summary.suppressed,
            "files_ignored": summary.files_ignored,
        },
    });
    pretty(&document)
}

/// SARIF 2.1.0, which GitHub code scanning and many editors read. Every configured rule is listed
/// (not only the ones that fired) so `ruleIndex` is stable between runs.
pub fn sarif(violations: &[Violation], rules: &BTreeMap<String, Rule>) -> String {
    let index: BTreeMap<&str, usize> = rules.keys().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let driver_rules: Vec<Value> = rules
        .iter()
        .map(|(id, rule)| {
            json!({
                "id": id,
                "shortDescription": { "text": rule.rationale.trim() },
                "help": { "text": rule.fix.trim() },
                "defaultConfiguration": { "level": "error" },
            })
        })
        .collect();
    let results: Vec<Value> = violations
        .iter()
        .map(|v| {
            json!({
                "ruleId": v.rule_id,
                "ruleIndex": index[v.rule_id.as_str()],
                "level": "error",
                "message": { "text": format!("{} Fix: {}", v.message, rules[&v.rule_id].fix.trim()) },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": v.path },
                        // SARIF positions start at 1.
                        "region": { "startLine": v.line.max(1), "startColumn": v.column.max(1) },
                    },
                }],
            })
        })
        .collect();
    let document = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": { "name": "prlint", "version": env!("CARGO_PKG_VERSION"), "rules": driver_rules } },
            "results": results,
        }],
    });
    pretty(&document)
}

fn pretty(document: &Value) -> String {
    serde_json::to_string_pretty(document).expect("a JSON value always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> BTreeMap<String, Rule> {
        let config = crate::config::Config::parse(
            "rules:\n  b-rule:\n    language: python\n    forbid_call: [x]\n    rationale: why b\n    fix: do b\n  a-rule:\n    language: python\n    forbid_call: [y]\n    rationale: why a\n    fix: do a\nscopes:\n  - rules: [a-rule, b-rule]\n    include: ['**']\n",
        )
        .unwrap();
        config.rules
    }

    fn violation(rule: &str, line: usize) -> Violation {
        Violation { path: "src/a.py".into(), line, column: 5, rule_id: rule.into(), message: "bad call".into() }
    }

    #[test]
    fn json_carries_rationale_and_fix() {
        let summary = Summary { files_checked: 3, files_with_violations: 1, suppressed: 2, files_ignored: 0 };
        let out: Value = serde_json::from_str(&json(&[violation("b-rule", 7)], &rules(), &summary)).unwrap();
        let first = &out["violations"][0];
        assert_eq!(first["rule"], "b-rule");
        assert_eq!((first["file"].as_str(), first["line"].as_u64(), first["column"].as_u64()), (Some("src/a.py"), Some(7), Some(5)));
        assert_eq!((first["rationale"].as_str(), first["fix"].as_str()), (Some("why b"), Some("do b")));
        assert_eq!(out["summary"]["violations"], 1);
        assert_eq!(out["summary"]["suppressed"], 2);
    }

    #[test]
    fn json_with_no_violations_has_an_empty_list() {
        let summary = Summary { files_checked: 0, files_with_violations: 0, suppressed: 0, files_ignored: 0 };
        let out: Value = serde_json::from_str(&json(&[], &rules(), &summary)).unwrap();
        assert_eq!(out["violations"], json!([]));
    }

    #[test]
    fn sarif_lists_all_rules_in_id_order_and_points_results_at_them() {
        let out: Value = serde_json::from_str(&sarif(&[violation("b-rule", 0)], &rules())).unwrap();
        assert_eq!(out["version"], "2.1.0");
        let run = &out["runs"][0];
        assert_eq!(run["tool"]["driver"]["rules"][0]["id"], "a-rule");
        assert_eq!(run["tool"]["driver"]["rules"][1]["id"], "b-rule");
        let result = &run["results"][0];
        assert_eq!(result["ruleIndex"], 1);
        assert_eq!(result["message"]["text"], "bad call Fix: do b");
        let region = &result["locations"][0]["physicalLocation"]["region"];
        assert_eq!((region["startLine"].as_u64(), region["startColumn"].as_u64()), (Some(1), Some(5)));
    }
}
