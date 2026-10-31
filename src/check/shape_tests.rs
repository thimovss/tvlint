//! Assignments, comparisons, call arguments, `allow_raise` and the literal checks.

use std::collections::BTreeMap;

use super::structure_tests::{list, lines, parse, py, run, ts};
use super::*;
use crate::config::{AllowRaise, ArgumentCheck, Lookup, Registry, Rule, Use};
use crate::config::{ArgumentKeys, ForbidLiteral, LiteralArgument};

// ---- assignments

#[test]
fn forbid_assignment_matches_the_target_as_written() {
    let src = "def f(aggregate, other):\n    aggregate.name = 'x'\n    aggregate.items[0] = 1\n    other.name = 'y'\n    aggregate.count += 1\n    local = 1\n";
    let rule = Rule { forbid_assignment: list(&["aggregate.*"]), ..py() };
    let found = run(("c.py", src), &[], &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 3, 5]);
    assert_eq!(found[0].1, "assigning to `aggregate.name` is forbidden");
    assert_eq!(found[2].1, "augmented assignment to `aggregate.count` is forbidden");
}

#[test]
fn forbid_assignment_in_apply_only_leaves_other_functions_alone() {
    let src = "class E:\n    def apply(self, a):\n        a.name = self.name\n\n    def describe(self, a):\n        a.name = 'x'\n";
    let everywhere = Rule { forbid_assignment: list(&["a.*"]), ..py() };
    assert_eq!(lines(("e.py", src), &[], &everywhere), [3, 6]);
    let outside_apply = Rule { forbid_assignment: list(&["a.*"]), within_function: list(&["*"]), except_function: list(&["apply"]), ..py() };
    assert_eq!(lines(("e.py", src), &[], &outside_apply), [6]);
}

#[test]
fn forbid_augmented_assignment_only_flags_in_place_changes() {
    let src = "def apply(self, a):\n    a.total = a.total + 1\n    a.total += 1\n    a.names.append('x')\n    a.total -= 1\n";
    let rule = Rule { forbid_augmented_assignment: list(&["a.*"]), ..py() };
    assert_eq!(lines(("e.py", src), &[], &rule), [3, 5]);
    let ts_src = "function apply(a) {\n  a.total = a.total + 1;\n  a.total += 1;\n}\n";
    let rule = Rule { forbid_augmented_assignment: list(&["a.*"]), ..ts() };
    assert_eq!(lines(("e.ts", ts_src), &[], &rule), [3]);
}

#[test]
fn typescript_assignments_cover_members_and_declarations() {
    let src = "this.name = 'x';\nagg.items[0] = 1;\nconst a = 1;\n";
    let rule = Rule { forbid_assignment: list(&["this.*", "agg.*", "a"]), ..ts() };
    assert_eq!(lines(("a.ts", src), &[], &rule), [1, 2]);
}

// ---- forbid_literal

#[test]
fn forbid_literal_flags_string_comparisons_on_either_side() {
    let src = "if x.status == 'active':\n    pass\nif 'draft' != x.status:\n    pass\nif x.status == Status.ACTIVE:\n    pass\nif x.n < 'z' or x.name in ('a', 'b') or x.f == f'{y}':\n    pass\n";
    let rule = Rule { forbid_literal: Some(ForbidLiteral { compared: true, ..Default::default() }), ..py() };
    let found = run(("c.py", src), &[], &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [1, 3]);
    assert_eq!(found[0].1, "compared with the string literal \"active\"; use an enum or a Literal type");
    let ts_src = "if (x.status === 'active') {}\nif (x.status === Status.Active) {}\nif (x.n < 'z') {}\n";
    let rule = Rule { forbid_literal: Some(ForbidLiteral { compared: true, ..Default::default() }), ..ts() };
    assert_eq!(lines(("a.ts", ts_src), &[], &rule), [1]);
}

#[test]
fn forbid_literal_assigned_to_constants() {
    let src = "STATUS_ACTIVE = 'active'\nMAX = 3\nname = 'x'\nclass C:\n    KIND = 'k'\n";
    let rule = Rule { forbid_literal: Some(ForbidLiteral { assigned_to: list(&["[A-Z]*"]), ..Default::default() }), ..py() };
    assert_eq!(lines(("c.py", src), &[], &rule), [1, 5]);
    let ts_src = "const STATUS = 'active';\nconst n = 'x';\nexport const LIMIT = 3;\n";
    let rule = Rule { forbid_literal: Some(ForbidLiteral { assigned_to: list(&["[A-Z]*"]), ..Default::default() }), ..ts() };
    assert_eq!(lines(("a.ts", ts_src), &[], &rule), [1]);
}

// ---- call arguments

fn args(calls: &[&str], names: &[&str]) -> Option<ArgumentCheck> {
    Some(ArgumentCheck { calls: list(calls), names: list(names) })
}

#[test]
fn require_argument_wants_the_keyword_on_every_matching_call() {
    let src = "a = AssetAggregate.from_aggregate_id(db, asset_id, account_id=account_id)\nb = AssetAggregate.from_aggregate_id(db, asset_id)\nc = AssetAggregate.from_aggregate_id(db, asset_id, **opts)\nd = other(db)\n";
    let rule = Rule { require_argument: args(&["*.from_aggregate_id"], &["account_id"]), ..py() };
    let found = run(("c.py", src), &[], &rule);
    assert_eq!(found, [(2, "the call to `AssetAggregate.from_aggregate_id` does not pass `account_id`".to_string())]);
}

#[test]
fn forbid_argument_flags_the_keyword_where_it_is_passed() {
    let src = "AggregateX.apply_event(db, ev, aggregate=asset, author='a')\nAggregateX.apply_events(db, evs, aggregate=asset)\n";
    let rule = Rule { forbid_argument: args(&["*.apply_event"], &["aggregate"]), ..py() };
    let found = run(("c.py", src), &[], &rule);
    assert_eq!(found, [(1, "the call to `AggregateX.apply_event` must not pass `aggregate`".to_string())]);
}

fn keys(calls: &[&str], forbid: &[&str]) -> ArgumentKeys {
    ArgumentKeys { calls: list(calls), forbid_keys: list(forbid), ..Default::default() }
}

#[test]
fn argument_keys_check_the_dict_in_a_position_or_a_keyword() {
    let src = "raise Err('a_failed.b', {'asset_name': n, 'asset_id': i})\nraise Err('a_failed.b', parameters={'tenantId': t})\nraise Err('a_failed.b', {k: v})\nraise Err('a_failed.b')\n";
    let rule = Rule { argument_keys: Some(ArgumentKeys { position: 1, ..keys(&["Err"], &["*_id", "*[A-Z]*"]) }), ..py() };
    let found = run(("c.py", src), &[], &rule);
    assert_eq!(found, [(1, "the key `asset_id` is not allowed in the dict passed to `Err`".to_string())]);
    let rule = Rule { argument_keys: Some(ArgumentKeys { keyword: Some("parameters".into()), ..keys(&["Err"], &["*[A-Z]*"]) }), ..py() };
    assert_eq!(lines(("c.py", src), &[], &rule), [2]);
}

#[test]
fn argument_keys_allow_keys_and_typescript_objects() {
    let src = "toast({ title: 'a', Bad: 1 });\ntoast({ title: 'a', ...rest });\n";
    let rule = Rule { argument_keys: Some(ArgumentKeys { calls: list(&["toast"]), allow_keys: list(&["[a-z]*"]), ..Default::default() }), ..ts() };
    let found = run(("a.ts", src), &[], &rule);
    assert_eq!(found.len(), 1);
    assert!(found[0].1.contains("`Bad`") && found[0].1.contains("[a-z]*"), "{found:?}");
}

// ---- allow_raise

fn allow(names: &[&str], subclass_of: Option<&str>) -> Option<AllowRaise> {
    Some(AllowRaise { names: list(names), subclass_of: subclass_of.map(String::from) })
}

#[test]
fn allow_raise_accepts_listed_names_and_subclasses_and_ignores_variables() {
    let base = ("exc.py", "class LocusException(Exception): ...\nclass InvalidCommand(LocusException): ...\nclass Other(Exception): ...\n");
    let src = "def f(e):\n    raise InvalidCommand('c')\n    raise LocusException('c')\n    raise ValueError('x')\n    raise Other('y')\n    raise e\n    raise\n    raise pkg.InvalidCommand('z')\n";
    let rule = Rule { allow_raise: allow(&["NotImplementedError"], Some("LocusException")), ..py() };
    let found = run(("c.py", src), &[base], &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [4, 5]);
    assert_eq!(found[0].1, "raising `ValueError` is not allowed here; allowed: NotImplementedError, LocusException and its subclasses");
    let only_names = Rule { allow_raise: allow(&["*Exception"], None), ..py() };
    assert_eq!(lines(("c.py", "raise ValueError('x')\nraise AppException('x')\nraise my.HttpException('x')\n"), &[], &only_names), [1]);
    let ts_rule = Rule { allow_raise: allow(&["AppError"], None), ..ts() };
    assert_eq!(lines(("a.ts", "throw new AppError('x');\nthrow new Error('y');\nthrow err;\n"), &[], &ts_rule), [2]);
}

// ---- literal checks

fn literal(calls: &[&str], configure: impl FnOnce(&mut LiteralArgument)) -> Rule {
    let mut arg = LiteralArgument { calls: list(calls), ..Default::default() };
    configure(&mut arg);
    Rule { literal_argument: Some(arg), ..py() }
}

#[test]
fn literal_argument_can_check_a_shape_without_a_registry() {
    let src = "raise Err('a_failed.b')\nraise Err('Bad Code')\nraise Err(code)\nraise Err(f'a_{x}')\n";
    let rule = literal(&["Err"], |a| a.pattern = list(&["[a-z]*_failed.[a-z]*"]));
    let found = run(("c.py", src), &[], &rule);
    assert_eq!(found, [(2, "`Bad Code` passed to `Err` does not match [a-z]*_failed.[a-z]*".to_string())]);
    let rule = literal(&["Err"], |a| a.require_literal = true);
    let found = run(("c.py", src), &[], &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [3, 4]);
    assert_eq!(found[0].1, "the value passed to `Err` must be a plain string literal, not an expression");
}

fn registry_files() -> [(&'static str, &'static str); 1] {
    [("locales.json", "{\"contract_creation_failed\": \"Could not create\", \"asset_rename_failed\": \"x\"}")]
}

fn with_registry(files: &[(&str, &str)], target: (&str, &str), rule: &Rule) -> Vec<(usize, String)> {
    let mut all: Vec<_> = files.to_vec();
    all.push(target);
    let parsed: Vec<_> = all.iter().map(|(p, t)| parse(p, t)).collect();
    let index = ClassIndex::build(&parsed);
    let compiled = CompiledRule::new("r", rule).unwrap();
    let docs: BTreeMap<_, _> = parsed.iter().filter(|f| f.language.is_data()).map(|f| (f.path.clone(), crate::data::Doc::build(f))).collect();
    let paths = Default::default();
    let ctx = Project { index: &index, aliases: &BTreeMap::new(), resolvers: &crate::adapters::Resolvers::default(), paths: &paths, docs: &docs };
    let mut out = Vec::new();
    check_file(parsed.last().unwrap(), &compiled, &ctx, &mut out);
    out.into_iter().map(|v| (v.line, v.message)).collect()
}

#[test]
fn lookup_prefix_checks_the_part_before_the_dot() {
    let src = "raise Err('contract_creation_failed.minimum_one_tenant')\nraise Err('contract_delete_failed.x')\nraise Err('plain')\n";
    let rule = literal(&["Err"], |a| {
        a.registry = Some(Registry { file: "locales.json".into(), path: "*".into(), use_: Use::Key });
        a.lookup = Lookup::Prefix;
    });
    let found = with_registry(&registry_files(), ("c.py", src), &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(found[0].1, "the prefix `contract_delete_failed` of `contract_delete_failed.x` passed to `Err` is not listed in locales.json");
}

#[test]
fn literal_assignment_checks_values_assigned_to_names() {
    let src = "class A:\n    event_type: ClassVar[str] = 'asset.renamed'\n    other = 'zzz'\n\n\nclass B:\n    event_type = make_type()\n";
    let codes = [("types.json", "[\"asset.renamed\"]")];
    let mut rule = Rule {
        literal_assignment: Some(LiteralArgument {
            names: list(&["event_type"]),
            registry: Some(Registry { file: "types.json".into(), path: "*".into(), use_: Use::Value }),
            ..Default::default()
        }),
        ..py()
    };
    assert!(with_registry(&codes, ("e.py", src), &rule).is_empty());
    rule.literal_assignment.as_mut().unwrap().pattern = list(&["*.created"]);
    let found = with_registry(&codes, ("e.py", src), &rule);
    assert_eq!(found, [(2, "`asset.renamed` assigned to `event_type` does not match *.created".to_string())]);
    rule.literal_assignment.as_mut().unwrap().pattern = Vec::new();
    rule.literal_assignment.as_mut().unwrap().require_literal = true;
    let found = with_registry(&codes, ("e.py", src), &rule);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [7]);
}
