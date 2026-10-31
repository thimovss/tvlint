//! `require_call`, `within_function`, and the class checks, in Python and in TypeScript.

use std::collections::{BTreeMap, HashSet};

use super::*;
use crate::config::{ClassFilter, Language, Rule};
use crate::source::{Parsers, SourceFile};

pub(super) fn parse(path: &str, text: &str) -> SourceFile {
    let (language, tree) = Parsers::new().unwrap().parse(path, text).unwrap();
    SourceFile { path: path.into(), language, text: text.into(), tree }
}

pub(super) fn list(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub(super) fn py() -> Rule {
    Rule { language: vec![Language::Python], ..Rule::default() }
}

pub(super) fn ts() -> Rule {
    Rule { language: vec![Language::TypeScript], ..Rule::default() }
}

/// (line, message) of every violation of `rule` in `target`, with `others` in the project for class lookups.
pub(super) fn run(target: (&str, &str), others: &[(&str, &str)], rule: &Rule) -> Vec<(usize, String)> {
    let files: Vec<SourceFile> = others.iter().chain(std::iter::once(&target)).map(|(p, t)| parse(p, t)).collect();
    let index = ClassIndex::build(&files);
    let compiled = CompiledRule::new("r", rule).unwrap();
    let (docs, paths) = (BTreeMap::new(), HashSet::new());
    let ctx = Project { index: &index, aliases: &BTreeMap::new(), resolvers: &crate::adapters::Resolvers::default(), paths: &paths, docs: &docs };
    let mut out = Vec::new();
    check_file(files.last().unwrap(), &compiled, &ctx, &mut out);
    out.into_iter().map(|v| (v.line, v.message)).collect()
}

pub(super) fn lines(target: (&str, &str), others: &[(&str, &str)], rule: &Rule) -> Vec<usize> {
    run(target, others, rule).into_iter().map(|h| h.0).collect()
}

// ---- require_call

#[test]
fn require_call_flags_a_file_without_the_call_at_line_one() {
    let rule = Rule { require_call: list(&["apply_event", "*.apply_event"]), ..py() };
    let without = ("c.py", "def rename(db):\n    db.add(1)\n");
    let found = run(without, &[], &rule);
    assert_eq!(found, [(1, "the file never calls `apply_event`, `*.apply_event`".to_string())]);
    assert!(run(("c.py", "def rename(db):\n    return Agg.apply_event(db)\n"), &[], &rule).is_empty());
    assert!(run(("c.py", "def rename(db):\n    apply_event(db)\n"), &[], &rule).is_empty());
}

#[test]
fn require_call_works_in_typescript_too() {
    let rule = Rule { require_call: list(&["register"]), ..ts() };
    assert_eq!(lines(("p.ts", "export const x = 1;\n"), &[], &rule), [1]);
    assert!(lines(("p.ts", "register({ name: 'a' });\n"), &[], &rule).is_empty());
}

#[test]
fn require_call_with_within_function_checks_each_selected_function() {
    let src = "def rename(db):\n    apply_event(db)\n\n\ndef broken(db):\n    db.add(1)\n\n\ndef _helper(db):\n    pass\n\n\nclass K:\n    def method(self):\n        return 1\n";
    let rule = Rule { require_call: list(&["apply_event"]), within_function: list(&["*"]), except_function: list(&["_*"]), ..py() };
    let found = run(("c.py", src), &[], &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [5, 14]);
    assert_eq!(found[0].1, "function `broken` never calls `apply_event`");
}

#[test]
fn require_call_per_function_counts_calls_in_nested_blocks() {
    let src = "def go(db, ok):\n    if ok:\n        for x in db:\n            apply_events(db, x)\n";
    let rule = Rule { require_call: list(&["apply_events"]), within_function: list(&["go"]), ..py() };
    assert!(run(("c.py", src), &[], &rule).is_empty());
}

#[test]
fn require_call_per_function_in_typescript_covers_methods_and_arrow_functions() {
    let src = "export function a() { save(); }\nexport function b() { }\nexport const c = () => { };\nclass K { m() { } }\n";
    let rule = Rule { require_call: list(&["save"]), within_function: list(&["*"]), ..ts() };
    assert_eq!(lines(("a.ts", src), &[], &rule), [2, 3, 4]);
}

// ---- within_function

#[test]
fn within_function_narrows_forbid_call() {
    let src = "def apply(self):\n    now = date.today()\n\n\ndef describe(self):\n    return date.today()\n";
    let rule = Rule { forbid_call: list(&["date.today"]), within_function: list(&["apply"]), ..py() };
    assert_eq!(lines(("e.py", src), &[], &rule), [2]);
    let everywhere = Rule { forbid_call: list(&["date.today"]), ..py() };
    assert_eq!(lines(("e.py", src), &[], &everywhere), [2, 6]);
}

#[test]
fn within_function_narrows_forbid_raise_and_respects_except() {
    let src = "def apply(self):\n    raise ValueError('x')\n\n\ndef _check(self):\n    raise ValueError('x')\n\n\nmodule_level = 1\nraise ValueError('y')\n";
    let rule = Rule { forbid_raise: list(&["ValueError"]), within_function: list(&["*"]), except_function: list(&["_*"]), ..py() };
    assert_eq!(lines(("e.py", src), &[], &rule), [2]);
}

// ---- class checks

const EVENT_BASE: (&str, &str) = ("domain_event.py", "class DomainEvent:\n    def apply(self, a): ...\n");

fn events() -> Rule {
    Rule { in_class: Some(ClassFilter { subclass_of: Some("DomainEvent".into()), names: Vec::new() }), ..py() }
}

#[test]
fn require_method_flags_subclasses_that_do_not_declare_it() {
    let src = "class A(DomainEvent[Agg]):\n    def apply(self, a):\n        return a\n\n\nclass B(DomainEvent[Agg]):\n    x: int\n\n\nclass Other:\n    pass\n";
    let rule = Rule { require_method: list(&["apply"]), ..events() };
    let found = run(("e.py", src), &[EVENT_BASE], &rule);
    assert_eq!(found, [(6, "class `B` does not declare the method `apply`".to_string())]);
}

#[test]
fn subclass_of_follows_inheritance_and_leaves_out_the_root() {
    let base = ("base.py", "class DomainEvent:\n    pass\n\n\nclass Mid(DomainEvent):\n    pass\n");
    let src = "class Leaf(Mid):\n    pass\n";
    let rule = Rule { require_method: list(&["apply"]), ..events() };
    assert_eq!(lines(("e.py", src), &[base], &rule), [1]);
    assert!(lines(("base.py", "class DomainEvent:\n    pass\n"), &[], &rule).is_empty());
}

#[test]
fn forbid_field_reports_each_field_where_it_is_declared() {
    let src = "class A(DomainEvent):\n    event_type: ClassVar[str] = 'a.a'\n    name: str\n    author: str\n    account_id: UUID = None\n";
    let rule = Rule { forbid_field: list(&["author", "date", "*_id"]), ..events() };
    let found = run(("e.py", src), &[EVENT_BASE], &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [4, 5]);
    assert_eq!(found[0].1, "class `A` must not declare the field `author`");
}

#[test]
fn require_field_accepts_annotated_and_assigned_names() {
    let src = "class A(Base):\n    account_id: UUID\n\n\nclass B(Base):\n    account_id = Column()\n\n\nclass C(Base):\n    def account_id(self): ...\n";
    let rule = Rule { require_field: list(&["account_id"]), ..py() };
    assert_eq!(lines(("t.py", src), &[], &rule), [9]);
}

#[test]
fn in_class_names_select_classes_by_glob() {
    let src = "class AssetEvent:\n    pass\n\n\nclass Helper:\n    pass\n";
    let rule = Rule { require_method: list(&["apply"]), in_class: Some(ClassFilter { subclass_of: None, names: list(&["*Event"]) }), ..py() };
    assert_eq!(lines(("e.py", src), &[], &rule), [1]);
}

#[test]
fn methods_and_fields_are_found_behind_decorators() {
    let src = "class A:\n    @property\n    def apply(self): ...\n    @staticmethod\n    def describe(): ...\n";
    let rule = Rule { require_method: list(&["apply", "describe", "other"]), ..py() };
    let found = run(("a.py", src), &[], &rule);
    assert_eq!(found.len(), 1);
    assert!(found[0].1.contains("`other`"));
}

#[test]
fn typescript_classes_have_methods_fields_and_a_separate_hierarchy() {
    let src = "export class A extends Base<T> {\n  id: string;\n  author = 'x';\n  apply() {}\n}\nexport class B extends Base {\n  run() {}\n}\nclass C {}\n";
    let base = ("base.ts", "export abstract class Base {}\n");
    let rule = Rule {
        require_method: list(&["apply"]),
        forbid_field: list(&["author"]),
        in_class: Some(ClassFilter { subclass_of: Some("Base".into()), names: Vec::new() }),
        ..ts()
    };
    let found = run(("a.ts", src), &[base], &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [3, 6]);
    // A Python class with the same name is not part of the TypeScript hierarchy.
    let python_base = ("base.py", "class Base:\n    pass\n\n\nclass Impl(Base):\n    pass\n");
    assert!(lines(("a.ts", "class Impl {}\n"), &[python_base], &rule).is_empty());
}

// ---- {n} placeholders

fn import_rule() -> Rule {
    Rule {
        language: vec![Language::Python],
        forbid_import: list(&["src/*/aggregate.py"]),
        except_import: list(&["src/{1}/**"]),
        ..Rule::default()
    }
}

fn import_hits(path: &str, source: &str, captures: &[String]) -> Vec<usize> {
    let file = parse(path, source);
    let index = ClassIndex::build(std::slice::from_ref(&file));
    let rule = import_rule();
    let compiled = CompiledRule::new("r", &rule).unwrap();
    let paths: HashSet<String> = ["src/assets/aggregate.py", "src/contracts/aggregate.py"].iter().map(|s| s.to_string()).collect();
    let resolvers = crate::adapters::Resolvers::for_test(&[], &paths.iter().cloned().collect::<Vec<_>>());
    let docs = BTreeMap::new();
    let ctx = Project { index: &index, aliases: &BTreeMap::new(), resolvers: &resolvers, paths: &paths, docs: &docs };
    let mut out = Vec::new();
    check_file_in_scope(&file, &compiled, &ctx, captures, &mut out);
    out.into_iter().map(|v| v.line).collect()
}

#[test]
fn placeholders_let_a_package_import_itself_only() {
    let src = "from assets.aggregate import AssetAggregate\nfrom contracts.aggregate import ContractAggregate\n";
    let assets = vec!["assets".to_string()];
    // `assets` may import its own aggregate; the contracts one is flagged.
    let lines = import_hits("src/assets/subscriptions/x.py", src, &assets);
    assert_eq!(lines, [2]);
}
