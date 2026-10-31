use super::*;

const GOOD: &str = r#"
rules:
  no-http:
    language: python
    forbid_raise: [HTTPException]
    rationale: why
    fix: how
scopes:
  - rules: [no-http]
    include: ["src/**"]
"#;

fn errors(yaml: &str) -> Vec<(String, String)> {
    let value: Value = serde_yaml_ng::from_str(yaml).unwrap();
    let mut report = Report::default();
    validate_structure(&value, &mut report);
    report.errors.into_iter().map(|d| (d.path, d.message)).collect()
}

fn has(errors: &[(String, String)], path: &str, fragment: &str) -> bool {
    errors.iter().any(|(p, m)| p == path && m.contains(fragment))
}

fn rule_errors(body: &str) -> Vec<(String, String)> {
    errors(&format!("rules:\n  r:\n{body}    rationale: why\n    fix: how\nscopes:\n  - rules: [r]\n    include: [\"**\"]\n"))
}

#[test]
fn accepts_data_rules() {
    assert!(rule_errors("    language: yaml\n    require_property: [jobs.*.timeout-minutes]\n").is_empty());
    assert!(rule_errors("    language: json\n    same_keys_as: en.json\n    same_placeholders: true\n").is_empty());
    assert!(rule_errors("    language: toml\n    property_value: { names: [\"dependencies.*\"], forbid_value: [\"[*]\"] }\n").is_empty());
    let literal = "    language: python\n    literal_argument:\n      calls: [\"*Exception\"]\n      registry: { file: codes.json, path: \"codes.*\", use: key }\n      report_unused: true\n";
    assert!(rule_errors(literal).is_empty());
}

#[test]
fn reports_data_rule_mistakes() {
    let found = rule_errors("    language: [json, python]\n    forbid_property: [\"a..b\"]\n");
    assert!(has(&found, "rules.r.language", "cannot share a rule"), "{found:?}");
    assert!(has(&found, "rules.r.forbid_property[0]", "empty segment"), "{found:?}");
    let found = rule_errors("    language: yaml\n    require_property: [\"jobs.*\"]\n    same_placeholders: true\n    forbid_call: [x]\n");
    assert!(has(&found, "rules.r.require_property[0]", "ends in a wildcard"), "{found:?}");
    assert!(has(&found, "rules.r.same_placeholders", "needs same_keys_as"), "{found:?}");
    assert!(has(&found, "rules.r.forbid_call", "not supported for yaml"), "{found:?}");
    let found = rule_errors("    language: python\n    literal_argument: { calls: [E], registry: { file: c.json, path: \"*\", use: keys } }\n");
    assert!(has(&found, "rules.r.literal_argument.registry.use", "expected key, value or path"), "{found:?}");
    let found = rule_errors("    language: python\n    require_property: [a.b]\n");
    assert!(has(&found, "rules.r.require_property", "not supported for python"), "{found:?}");
}

#[test]
fn typed_loader_checks_data_property_values() {
    let load = |body: &str| {
        Config::parse(&format!("rules:\n  r:\n{body}    rationale: why\n    fix: how\nscopes:\n  - rules: [r]\n    include: [\"**\"]\n"))
            .err()
            .map(|e| format!("{e:#}"))
    };
    assert!(load("    language: json\n    property_value: { names: [a], forbid_literal: true }\n").unwrap().contains("not forbid_literal"));
    assert!(load("    language: json\n    property_value: { names: [a] }\n").unwrap().contains("needs forbid_value or require_value"));
    assert!(load("    language: typescript\n    property_value: { names: [a], forbid_value: [x] }\n").unwrap().contains("read JSON, YAML and TOML"));
    assert!(load("    language: json\n    property_value: { names: [a], require_value: [x] }\n").is_none());
}

#[test]
fn accepts_a_good_config() {
    assert!(errors(GOOD).is_empty());
}

#[test]
fn reports_every_problem_in_one_pass() {
    let yaml = GOOD
        .replace("forbid_raise", "forbid_rase")
        .replace("    fix: how\n", "")
        .replace("rules: [no-http]", "rules: [no-htp]");
    let found = errors(&yaml);
    assert!(has(&found, "rules.no-http.forbid_rase", "unknown key"), "{found:?}");
    assert!(has(&found, "rules.no-http", "missing required key `fix`"), "{found:?}");
    assert!(has(&found, "rules.no-http", "no checks"), "{found:?}");
    assert!(has(&found, "scopes[0].rules[0]", "unknown rule `no-htp`"), "{found:?}");
}

#[test]
fn flags_wrong_types_and_bad_globs() {
    let yaml = GOOD.replace("[HTTPException]", "HTTPException").replace("\"src/**\"", "\"src/[\"");
    let found = errors(&yaml);
    assert!(has(&found, "rules.no-http.forbid_raise", "expected a list"), "{found:?}");
    assert!(has(&found, "scopes[0].include[0]", "invalid glob"), "{found:?}");
}

#[test]
fn rejects_unsupported_language_with_suggestion() {
    let found = errors(&GOOD.replace("language: python", "language: pyhton"));
    assert!(has(&found, "rules.no-http.language", "unsupported language"), "{found:?}");
}

#[test]
fn filename_rule_needs_no_language() {
    let yaml = r#"
rules:
  names:
    require_filename: ["test_*.py", conftest.py]
    rationale: r
    fix: f
scopes:
  - rules: [names]
    include: ["tests/**"]
"#;
    assert!(errors(yaml).is_empty(), "{:?}", errors(yaml));
}

#[test]
fn code_checks_still_need_a_language() {
    let found = errors(&GOOD.replace("    language: python\n", ""));
    assert!(has(&found, "rules.no-http", "missing required key `language`"), "{found:?}");
}

#[test]
fn language_may_be_a_list() {
    let yaml = GOOD.replace("language: python", "language: [javascript, typescript]");
    assert!(errors(&yaml).is_empty(), "{:?}", errors(&yaml));
    let found = errors(&GOOD.replace("language: python", "language: [typescript, typscript]"));
    assert!(has(&found, "rules.no-http.language[1]", "unsupported language"), "{found:?}");
}

#[test]
fn forbid_catch_is_rejected_outside_python() {
    let yaml = GOOD.replace("language: python", "language: typescript").replace(
        "forbid_raise: [HTTPException]",
        "forbid_catch: { subclass_of: AppError }",
    );
    let found = errors(&yaml);
    assert!(has(&found, "rules.no-http.forbid_catch", "not supported for typescript"), "{found:?}");
    let ok = GOOD.replace("forbid_raise: [HTTPException]", "forbid_catch: { subclass_of: AppError }");
    assert!(errors(&ok).is_empty());
}

const ES_RULE: &str = r#"
rules:
  r:
    language: typescript
    rationale: why
    fix: how
scopes:
  - rules: [r]
    include: ["src/**"]
"#;

fn es(extra: &str) -> Vec<(String, String)> {
    errors(&ES_RULE.replace("    fix: how\n", &format!("    fix: how\n{extra}")))
}

#[test]
fn accepts_the_javascript_checks() {
    for extra in [
        "    forbid_import: [\"@base-ui/react/**\"]\n    except_import: [\"src/ui/**\"]\n",
        "    forbid_element: [button, input]\n",
        "    forbid_property: [onError]\n",
        "    forbid_call: [toast.add]\n    within_property: [onError]\n",
        "    property_value: { names: queryKey, array_starts_with: accountID }\n",
        "    property_value: { names: [title], in_call: toast.add, forbid_literal: true }\n",
    ] {
        assert!(es(extra).is_empty(), "{extra}: {:?}", es(extra));
    }
}

#[test]
fn rejects_bad_javascript_checks() {
    let found = es("    except_import: [\"src/**\"]\n");
    assert!(has(&found, "rules.r.except_import", "nothing to except"), "{found:?}");
    let found = es("    forbid_import: [\"src/[\"]\n");
    assert!(has(&found, "rules.r.forbid_import[0]", "invalid glob"), "{found:?}");
    let found = es("    within_property: [onError]\n    forbid_property: [x]\n");
    assert!(has(&found, "rules.r.within_property", "nothing to narrow"), "{found:?}");
    let found = es("    property_value: { names: queryKey }\n");
    assert!(has(&found, "rules.r.property_value", "no constraint"), "{found:?}");
    let found = es("    property_value: { array_starts_with: a }\n");
    assert!(has(&found, "rules.r.property_value", "missing required key `names`"), "{found:?}");
    let found = es("    property_value: { names: x, forbid_litral: true }\n");
    assert!(has(&found, "rules.r.property_value.forbid_litral", "unknown key"), "{found:?}");
}

#[test]
fn javascript_checks_are_rejected_for_python() {
    let yaml = ES_RULE.replace("language: typescript", "language: [python, typescript]").replace(
        "    fix: how\n",
        "    fix: how\n    forbid_element: [x]\n",
    );
    let found = errors(&yaml);
    assert!(has(&found, "rules.r.forbid_element", "not supported for python"), "{found:?}");
}

#[test]
fn rule_ids_must_be_lowercase_words_with_dashes() {
    let yaml = GOOD.replace("no-http", "web/My_rule");
    assert!(has(&errors(&yaml), "rules.web/My_rule", "must be lowercase words joined by dashes"), "{:?}", errors(&yaml));
}

#[test]
fn aliases_must_agree_on_the_trailing_slash() {
    let yaml = format!("aliases:\n  \"@/\": src\n{GOOD}");
    let found = errors(&yaml);
    assert!(has(&found, "aliases.@/", "must both end with"), "{found:?}");
    assert!(errors(&format!("aliases:\n  \"@/\": src/\n{GOOD}")).is_empty());
}

#[test]
fn rust_rules_accept_calls_and_imports() {
    for extra in [
        "    forbid_call: [\"*.unwrap\", \"println!\"]\n",
        "    forbid_import: [\"sqlx\", \"sqlx::**\"]\n    except_import: [\"crate::db::**\"]\n",
    ] {
        let yaml = ES_RULE.replace("language: typescript", "language: rust").replace("    fix: how\n", &format!("    fix: how\n{extra}"));
        assert!(errors(&yaml).is_empty(), "{extra}: {:?}", errors(&yaml));
    }
}

#[test]
fn rust_has_no_exceptions_or_jsx() {
    let rust = |extra: &str| {
        errors(&ES_RULE.replace("language: typescript", "language: rust").replace("    fix: how\n", &format!("    fix: how\n{extra}")))
    };
    let found = rust("    forbid_raise: [Error]\n");
    assert!(has(&found, "rules.r.forbid_raise", "not supported for rust"), "{found:?}");
    let found = rust("    forbid_catch: { subclass_of: E }\n");
    assert!(has(&found, "rules.r.forbid_catch", "not supported for rust"), "{found:?}");
    let found = rust("    forbid_element: [button]\n");
    assert!(has(&found, "rules.r.forbid_element", "not supported for rust"), "{found:?}");
    let mixed = errors(&ES_RULE.replace("language: typescript", "language: [typescript, rust]").replace("    fix: how\n", "    fix: how\n    forbid_raise: [E]\n"));
    assert!(has(&mixed, "rules.r.forbid_raise", "not supported for rust"), "{mixed:?}");
    assert!(!has(&mixed, "rules.r.forbid_raise", "not supported for typescript"), "{mixed:?}");
}

#[test]
fn go_accepts_calls_and_imports_but_has_no_exceptions() {
    let go = |extra: &str| {
        errors(&ES_RULE.replace("language: typescript", "language: go").replace("    fix: how\n", &format!("    fix: how\n{extra}")))
    };
    assert!(go("    forbid_call: [\"fmt.Println\", panic]\n").is_empty());
    assert!(go("    forbid_import: [\"app/internal/db\"]\n").is_empty());
    let found = go("    forbid_raise: [Error]\n");
    assert!(has(&found, "rules.r.forbid_raise", "not supported for go"), "{found:?}");
    let found = go("    forbid_element: [button]\n");
    assert!(has(&found, "rules.r.forbid_element", "not supported for go"), "{found:?}");
}

#[test]
fn css_accepts_functions_imports_and_declarations_only() {
    let css = |extra: &str| {
        errors(&ES_RULE.replace("language: typescript", "language: css").replace("    fix: how\n", &format!("    fix: how\n{extra}")))
    };
    assert!(css("    forbid_call: [rgb]\n").is_empty());
    assert!(css("    forbid_import: [\"https://**\"]\n").is_empty());
    assert!(css("    forbid_property: [float]\n").is_empty());
    let found = css("    forbid_element: [button]\n");
    assert!(has(&found, "rules.r.forbid_element", "not supported for css"), "{found:?}");
    let found = css("    property_value: { names: [color], forbid_literal: true }\n");
    assert!(has(&found, "rules.r.property_value", "not supported for css"), "{found:?}");
}

#[test]
fn max_lines_must_be_a_positive_number_and_needs_a_language() {
    let ml = |extra: &str| {
        errors(&ES_RULE.replace("    fix: how\n", &format!("    fix: how\n{extra}")))
    };
    assert!(ml("    max_lines: 1000\n").is_empty(), "{:?}", ml("    max_lines: 1000\n"));
    for bad in ["0", "many", "-5", "1.5"] {
        let found = ml(&format!("    max_lines: {bad}\n"));
        assert!(has(&found, "rules.r.max_lines", "positive whole number"), "{bad}: {found:?}");
    }
}

#[test]
fn shell_accepts_commands_and_sources_but_nothing_syntactic() {
    let sh = |extra: &str| {
        errors(&ES_RULE.replace("language: typescript", "language: shell").replace("    fix: how\n", &format!("    fix: how\n{extra}")))
    };
    assert!(sh("    forbid_call: [curl, \"sudo\"]\n").is_empty());
    assert!(sh("    forbid_import: [\"/etc/**\"]\n").is_empty());
    let found = sh("    forbid_raise: [Error]\n");
    assert!(has(&found, "rules.r.forbid_raise", "not supported for shell"), "{found:?}");
    let found = sh("    forbid_element: [button]\n");
    assert!(has(&found, "rules.r.forbid_element", "not supported for shell"), "{found:?}");
}

#[test]
fn html_accepts_elements_and_attributes_only() {
    let html = |extra: &str| {
        errors(&ES_RULE.replace("language: typescript", "language: html").replace("    fix: how\n", &format!("    fix: how\n{extra}")))
    };
    assert!(html("    forbid_element: [font, center]\n").is_empty());
    assert!(html("    forbid_property: [onclick, style]\n").is_empty());
    let found = html("    forbid_call: [alert]\n");
    assert!(has(&found, "rules.r.forbid_call", "not supported for html"), "{found:?}");
    let found = html("    forbid_import: [x]\n");
    assert!(has(&found, "rules.r.forbid_import", "not supported for html"), "{found:?}");
}

fn schema_keys(node: &Value) -> BTreeSet<String> {
    node.get("properties").and_then(Value::as_mapping).unwrap().keys().map(|k| k.as_str().unwrap().to_string()).collect()
}

fn known(keys: &[(&str, &str)]) -> BTreeSet<String> {
    keys.iter().map(|(k, _)| k.to_string()).collect()
}

#[test]
fn json_schema_lists_the_same_keys_as_validate() {
    let schema: Value = serde_yaml_ng::from_str(crate::SCHEMA).unwrap();
    let defs = schema.get("$defs").unwrap();
    assert_eq!(schema_keys(&schema), known(TOP_KEYS));
    assert_eq!(schema_keys(defs.get("rule").unwrap()), known(RULE_KEYS));
    assert_eq!(schema_keys(defs.get("scope").unwrap()), known(SCOPE_KEYS));
    let languages: BTreeSet<String> = defs
        .get("language")
        .and_then(|l| l.get("enum"))
        .and_then(Value::as_sequence)
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    assert_eq!(languages, LANGUAGES.iter().map(|l| l.to_string()).collect());
}

#[test]
fn suggests_close_names() {
    assert_eq!(closest("forbid_rase", ["forbid_raise", "fix"].into_iter()), Some("forbid_raise"));
    assert_eq!(closest("zzzzzz", ["forbid_raise"].into_iter()), None);
}

#[test]
fn missing_sections_are_reported() {
    let found = errors("rules: {}\n");
    assert!(has(&found, "", "missing required key `scopes`"), "{found:?}");
}

#[test]
fn an_empty_config_is_only_a_warning() {
    let value: Value = serde_yaml_ng::from_str("rules: {}\nscopes: []\n").unwrap();
    let mut report = Report::default();
    validate_structure(&value, &mut report);
    assert!(report.errors.is_empty());
    assert_eq!(report.warnings.len(), 2);
}
