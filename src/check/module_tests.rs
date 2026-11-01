//! `max_functions`, `forbid_export` and `require_export`.

use super::structure_tests::{list, lines, py, run, ts};
use crate::config::{ForbidExport, MaxFunctions, RequireExport, Rule};

fn components() -> Rule {
    Rule { max_functions: Some(MaxFunctions { names: list(&["[A-Z]*"]), containing_element: list(&["*"]), max: 1 }), ..ts() }
}

// ---- max_functions

#[test]
fn a_second_component_in_a_file_is_reported() {
    let src = "function A() { return <div/>; }\nconst B = () => <span/>;\nfunction C() { return <p/>; }\n";
    let found = run(("a.tsx", src), &[], &components());
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 3]);
    assert!(found[0].1.contains("`B` is function 2 of 3"), "{found:?}");
    assert!(found[0].1.contains("at most 1 allowed"), "{found:?}");
}

#[test]
fn helpers_and_nested_functions_are_not_components() {
    let src = "function toLabel(x) { return x; }\nfunction format() { return <b/>; }\nconst A = () => {\n  const Inner = () => <i/>;\n  return <div>{Inner()}</div>;\n};\nfunction plain() { return 1; }\n";
    // `format` is lowercase and `Inner` is nested, so only `A` counts.
    assert!(lines(("a.tsx", src), &[], &components()).is_empty());
}

#[test]
fn a_capitalised_function_without_markup_does_not_count() {
    let src = "function A() { return <div/>; }\nfunction Helper() { return 1; }\n";
    assert!(lines(("a.tsx", src), &[], &components()).is_empty());
}

#[test]
fn methods_of_a_class_are_not_top_level_functions() {
    let src = "function A() { return <div/>; }\nclass K { Render() { return <p/>; } }\n";
    assert!(lines(("a.tsx", src), &[], &components()).is_empty());
}

#[test]
fn max_functions_without_an_element_counts_every_matching_function() {
    let rule = Rule { max_functions: Some(MaxFunctions { names: Vec::new(), containing_element: Vec::new(), max: 1 }), ..py() };
    let src = "def one():\n    pass\n\n\ndef two():\n    pass\n";
    assert_eq!(lines(("m.py", src), &[], &rule), [5]);
}

#[test]
fn max_zero_forbids_the_functions_outright() {
    let rule = Rule { max_functions: Some(MaxFunctions { names: list(&["get*"]), containing_element: Vec::new(), max: 0 }), ..py() };
    assert_eq!(lines(("m.py", "def get_x():\n    pass\n\n\ndef set_x():\n    pass\n"), &[], &rule), [1]);
}

// ---- exports

fn no_declared_exports() -> Rule {
    Rule { forbid_export: Some(ForbidExport { names: list(&["[A-Z]*"]), declared: true, default: None }), ..ts() }
}

#[test]
fn declared_exports_are_forbidden_but_export_default_name_is_not() {
    let src = "export function A() {}\nexport const B = () => 1;\nexport default function C() {}\nfunction D() {}\nexport default D;\nexport { D as E };\nexport const lower = 1;\n";
    let found = run(("a.tsx", src), &[], &no_declared_exports());
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(found[0].1, "exporting `A` as a named export where it is declared is forbidden");
    assert!(found[2].1.contains("as a default export"), "{found:?}");
}

#[test]
fn types_are_not_exports_that_count() {
    let src = "export interface Props { a: number }\nexport type Id = string;\nexport type { Props as P };\n";
    assert!(lines(("a.tsx", src), &[], &no_declared_exports()).is_empty());
}

#[test]
fn forbid_export_can_target_only_default_or_only_named() {
    let src = "export const A = 1;\nexport default A;\n";
    let default_only = Rule { forbid_export: Some(ForbidExport { names: Vec::new(), declared: false, default: Some(true) }), ..ts() };
    assert_eq!(lines(("a.ts", src), &[], &default_only), [2]);
    let named_only = Rule { forbid_export: Some(ForbidExport { names: Vec::new(), declared: false, default: Some(false) }), ..ts() };
    assert_eq!(lines(("a.ts", src), &[], &named_only), [1]);
}

#[test]
fn exports_with_several_names_are_each_checked() {
    let rule = Rule { forbid_export: Some(ForbidExport { names: list(&["b"]), declared: false, default: None }), ..ts() };
    let found = run(("a.ts", "const a = 1, b = 2;\nexport { a, b };\nexport * from './x';\n"), &[], &rule);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].1.contains("`b`"), "{found:?}");
}

fn needs_default() -> Rule {
    Rule { require_export: Some(RequireExport { names: list(&["[A-Z]*"]), default: true }), ..ts() }
}

#[test]
fn require_export_wants_a_default_export_named_like_a_component() {
    let ok = "function Button() {}\nexport default Button;\n";
    assert!(lines(("b.tsx", ok), &[], &needs_default()).is_empty());
    assert!(lines(("b.tsx", "export default function Button() {}\n"), &[], &needs_default()).is_empty());
    assert!(lines(("b.tsx", "function B() {}\nexport { B as default };\n"), &[], &needs_default()).is_empty());

    let named = run(("b.tsx", "export const Button = 1;\n"), &[], &needs_default());
    assert_eq!(named, [(1, "the file has no default export named [A-Z]*".to_string())]);
}

#[test]
fn require_export_reports_a_file_with_no_exports_or_the_wrong_name() {
    assert_eq!(lines(("b.tsx", "const x = 1;\n"), &[], &needs_default()), [1]);
    assert_eq!(lines(("b.tsx", "const button = 1;\nexport default button;\n"), &[], &needs_default()), [1]);
    // An expression is named as written.
    assert_eq!(lines(("b.tsx", "export default memo(Button);\n"), &[], &needs_default()), [1]);
}
