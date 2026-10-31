use super::*;
use crate::config::{ClassSelector, Language};
use crate::data::Doc;
use crate::source::Parsers;

fn parse(path: &str, text: &str) -> SourceFile {
    let (language, tree) = Parsers::new().unwrap().parse(path, text).unwrap();
    SourceFile { path: path.into(), language, text: text.into(), tree }
}

fn rule(language: Language, raise: &[&str], call: &[&str], catch: Option<&str>) -> Rule {
    Rule {
        language: vec![language],
        forbid_raise: raise.iter().map(|s| s.to_string()).collect(),
        forbid_call: call.iter().map(|s| s.to_string()).collect(),
        forbid_catch: catch.map(|c| ClassSelector { subclass_of: c.into() }),
        ..empty_rule()
    }
}

fn empty_rule() -> Rule {
    Rule {
        language: Vec::new(),
        forbid_raise: Vec::new(),
        forbid_call: Vec::new(),
        forbid_catch: None,
        forbid_import: Vec::new(),
        except_import: Vec::new(),
        forbid_element: Vec::new(),
        forbid_property: Vec::new(),
        property_value: None,
        within_property: Vec::new(),
        require_property: Vec::new(),
        same_keys_as: None,
        same_placeholders: false,
        literal_argument: None,
        require_filename: Vec::new(),
        max_lines: None,
        rationale: String::new(),
        fix: String::new(),
    }
}

/// Lines (1-based) of every violation of `rule` in `source`, parsed as the file `path`.
fn lines(path: &str, source: &str, rule: &Rule) -> Vec<usize> {
    hits(path, source, rule).into_iter().map(|h| h.0).collect()
}

fn hits(path: &str, source: &str, rule: &Rule) -> Vec<(usize, String)> {
    hits_with(path, source, rule, &BTreeMap::new(), &HashSet::new())
}

fn hits_with(
    path: &str,
    source: &str,
    rule: &Rule,
    aliases: &BTreeMap<String, String>,
    paths: &HashSet<String>,
) -> Vec<(usize, String)> {
    let file = parse(path, source);
    let index = ClassIndex::build(std::slice::from_ref(&file));
    let compiled = CompiledRule::new("r", rule).unwrap();
    let docs = BTreeMap::new();
    let ctx = Project { index: &index, aliases, modules: &crate::modules::Modules::default(), paths, docs: &docs };
    let mut out = Vec::new();
    check_file(&file, &compiled, &ctx, &mut out);
    out.into_iter().map(|v| (v.line, v.message)).collect()
}

/// Checks `target` among `files` (path, text), all visible to the rule as a project.
fn project_hits(files: &[(&str, &str)], target: &str, rule: &Rule) -> Vec<(usize, usize, String)> {
    let parsed: Vec<SourceFile> = files.iter().map(|(p, t)| parse(p, t)).collect();
    let index = ClassIndex::build(&parsed);
    let docs: BTreeMap<String, Doc> = parsed.iter().filter(|f| f.language.is_data()).map(|f| (f.path.clone(), Doc::build(f))).collect();
    let compiled = CompiledRule::new("r", rule).unwrap();
    let (aliases, paths) = (BTreeMap::new(), HashSet::new());
    let ctx = Project { index: &index, aliases: &aliases, modules: &crate::modules::Modules::default(), paths: &paths, docs: &docs };
    let mut out = Vec::new();
    check_file(parsed.iter().find(|f| f.path == target).unwrap(), &compiled, &ctx, &mut out);
    out.into_iter().map(|v| (v.line, v.column, v.message)).collect()
}

fn data_rule(language: Language) -> Rule {
    Rule { language: vec![language], ..empty_rule() }
}

fn property_value(names: &[&str], forbid: &[&str], require: &[&str]) -> Option<crate::config::PropertyValue> {
    let list = |items: &[&str]| items.iter().map(|s| s.to_string()).collect();
    Some(crate::config::PropertyValue {
        names: list(names),
        in_call: Vec::new(),
        forbid_literal: false,
        array_starts_with: None,
        forbid_value: list(forbid),
        require_value: list(require),
    })
}

// ---- JSON, YAML, TOML

#[test]
fn forbid_property_reports_the_key_at_its_path() {
    let json = "{\n  \"scripts\": {\n    \"postinstall\": \"x\",\n    \"build\": \"y\"\n  },\n  \"postinstall\": 1\n}\n";
    let r = Rule { forbid_property: vec!["scripts.postinstall".into()], ..data_rule(Language::Json) };
    let found = project_hits(&[("package.json", json)], "package.json", &r);
    assert_eq!(found, [(3, 5, "the `scripts.postinstall` property is forbidden".to_string())]);
    let yaml = "jobs:\n  a:\n    permissions: write-all\n  b:\n    runs-on: x\n";
    let r = Rule { forbid_property: vec!["jobs.*.permissions".into()], ..data_rule(Language::Yaml) };
    assert_eq!(project_hits(&[("w.yml", yaml)], "w.yml", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
    let toml = "[dependencies]\nserde = \"1\"\n[dev-dependencies]\nserde = \"1\"\n";
    let r = Rule { forbid_property: vec!["dev-dependencies".into()], ..data_rule(Language::Toml) };
    assert_eq!(project_hits(&[("Cargo.toml", toml)], "Cargo.toml", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
}

#[test]
fn forbid_value_and_require_value_read_scalars_by_path() {
    let json = "{\"dependencies\": {\"a\": \"^1.0.0\", \"b\": \"1.2.3\", \"c\": \"latest\"}, \"strict\": false}";
    let r = Rule { property_value: property_value(&["dependencies.*"], &["^*", "~*", "latest"], &[]), ..data_rule(Language::Json) };
    let found = project_hits(&[("p.json", json)], "p.json", &r);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found[0].2.contains("`dependencies.a` is `^1.0.0`") && found[0].2.contains("`^*`"), "{found:?}");
    let r = Rule { property_value: property_value(&["strict"], &[], &["true"]), ..data_rule(Language::Json) };
    let found = project_hits(&[("p.json", json)], "p.json", &r);
    assert_eq!(found.len(), 1);
    assert!(found[0].2.contains("`strict` is `false`; it must match `true`"), "{found:?}");
}

#[test]
fn yaml_values_ignore_quotes_and_walk_lists() {
    let yaml = "services:\n  web:\n    image: \"app:latest\"\n  db:\n    image: pg:16\nsteps:\n  - uses: a/b@main\n  - uses: c/d@0123abc\n";
    let r = Rule { property_value: property_value(&["services.*.image"], &["*:latest"], &[]), ..data_rule(Language::Yaml) };
    assert_eq!(project_hits(&[("c.yaml", yaml)], "c.yaml", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
    let r = Rule { property_value: property_value(&["steps.*.uses"], &[], &["*@[0-9a-f][0-9a-f][0-9a-f]*"]), ..data_rule(Language::Yaml) };
    assert_eq!(project_hits(&[("c.yaml", yaml)], "c.yaml", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [7]);
}

#[test]
fn require_property_checks_every_match_of_the_parent() {
    let yaml = "jobs:\n  build:\n    timeout-minutes: 5\n  test:\n    runs-on: x\n";
    let r = Rule { require_property: vec!["jobs.*.timeout-minutes".into()], ..data_rule(Language::Yaml) };
    let found = project_hits(&[("w.yml", yaml)], "w.yml", &r);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].0, found[0].2.as_str()), (4, "`jobs.test` is missing the required `timeout-minutes` property"));
    // A literal parent that is absent is reported once; a wildcard parent with no match is fine.
    let r = Rule { require_property: vec!["permissions.contents".into(), "services.*.image".into()], ..data_rule(Language::Yaml) };
    let found = project_hits(&[("w.yml", yaml)], "w.yml", &r);
    assert_eq!(found.len(), 1);
    assert!(found[0].2.contains("`permissions.contents` is missing"), "{found:?}");
    // The root counts as a parent, and every YAML document is checked.
    let r = Rule { require_property: vec!["name".into()], ..data_rule(Language::Yaml) };
    let found = project_hits(&[("k.yaml", "name: a\n---\nkind: b\n")], "k.yaml", &r);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
    let r = Rule { require_property: vec!["package.license".into()], ..data_rule(Language::Toml) };
    assert!(project_hits(&[("Cargo.toml", "[package]\nname = \"x\"\nlicense = \"MIT\"\n")], "Cargo.toml", &r).is_empty());
}

fn locales(rule_placeholders: bool) -> Vec<(usize, usize, String)> {
    let en = "{\"title\": \"Hi {{name}}\", \"menu\": {\"home\": \"Home\", \"about\": \"About\"}}";
    let nl = "{\n  \"title\": \"Hallo\",\n  \"menu\": {\n    \"home\": \"Thuis\",\n    \"extra\": \"x\"\n  }\n}\n";
    let r = Rule { same_keys_as: Some("en.json".into()), same_placeholders: rule_placeholders, ..data_rule(Language::Json) };
    project_hits(&[("en.json", en), ("nl.json", nl)], "nl.json", &r)
}

#[test]
fn same_keys_as_reports_missing_and_extra_keys() {
    let found = locales(false);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(found.iter().any(|h| h.2 == "`menu.about` is missing; en.json has it" && h.0 == 3));
    assert!(found.iter().any(|h| h.2 == "`menu.extra` is not in en.json" && h.0 == 5));
}

#[test]
fn same_placeholders_compares_shared_messages() {
    let found = locales(true);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found.iter().any(|h| h.0 == 2 && h.2.contains("has placeholders none, but en.json has {name}")));
}

#[test]
fn the_reference_file_is_not_compared_with_itself() {
    let r = Rule { same_keys_as: Some("./en.json".into()), ..data_rule(Language::Json) };
    assert!(project_hits(&[("en.json", "{\"a\": 1}")], "en.json", &r).is_empty());
}

// ---- Literal arguments against a registry

fn literal_rule_for(language: Language, calls: &[&str], position: usize, use_: Use, path: &str) -> Rule {
    Rule {
        language: vec![language],
        literal_argument: Some(crate::config::LiteralArgument {
            calls: calls.iter().map(|s| s.to_string()).collect(),
            position,
            registry: crate::config::Registry { file: "codes.json".into(), path: path.into(), use_ },
            report_unused: true,
        }),
        ..empty_rule()
    }
}

#[test]
fn literal_arguments_must_be_in_the_registry() {
    let codes = ("codes.json", "{\"codes\": [\"a_failed.x\", \"b_failed.y\"]}");
    let py = "raise Err('a_failed.x', {})\nraise Err('zzz', {})\nraise Err(code)\nraise Err(f'a{x}')\nraise Err(message='q')\nraise Other('nope')\n";
    let r = literal_rule_for(Language::Python, &["Err"], 0, Use::Value, "codes.*");
    let found = project_hits(&[codes, ("a.py", py)], "a.py", &r);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].0, found[0].2.as_str()), (2, "`zzz` passed to `Err` is not listed in codes.json"));
    let ts = "new Err('a_failed.x');\nthrow new Err(`b_failed.y`);\nnew Err('nope');\nnew Err(...rest, 'nope');\n";
    let r = literal_rule_for(Language::TypeScript, &["Err"], 0, Use::Value, "codes.*");
    assert_eq!(project_hits(&[codes, ("a.ts", ts)], "a.ts", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
    let rs = "fn f() { err(\"a_failed.x\", 1); err(\"bad\", 2); err(name, 3); }\n";
    let r = literal_rule_for(Language::Rust, &["err"], 0, Use::Value, "codes.*");
    assert_eq!(project_hits(&[codes, ("a.rs", rs)], "a.rs", &r).len(), 1);
}

#[test]
fn a_later_argument_and_registry_keys() {
    let codes = ("codes.json", "{\"codes\": {\"home\": \"Home\", \"nav\": {\"about\": \"About\"}}}");
    let ts = "t('x', 'home');\nt('x', 'nope');\n";
    let r = literal_rule_for(Language::TypeScript, &["t"], 1, Use::Key, "codes.*");
    assert_eq!(project_hits(&[codes, ("a.ts", ts)], "a.ts", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [2]);
    let ts = "t('nav.about');\nt('nav');\nt('home');\n";
    let r = literal_rule_for(Language::TypeScript, &["t"], 0, Use::Path, "codes.**");
    assert_eq!(project_hits(&[codes, ("a.ts", ts)], "a.ts", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2, 3]);
    let r = literal_rule_for(Language::TypeScript, &["t"], 0, Use::Path, "**");
    assert_eq!(project_hits(&[codes, ("a.ts", ts)], "a.ts", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2, 3]);
}

#[test]
fn unused_registry_entries_are_found_across_files() {
    let json = "{\n  \"codes\": [\n    \"a\",\n    \"b\",\n    \"c\"\n  ]\n}\n";
    let files = [parse("codes.json", json), parse("x.py", "Err('a')\n"), parse("y.py", "Err('c')\nErr('c')\n")];
    let docs: BTreeMap<String, Doc> = [("codes.json".to_string(), Doc::build(&files[0]))].into();
    let r = literal_rule_for(Language::Python, &["Err"], 0, Use::Value, "codes.*");
    let compiled = CompiledRule::new("r", &r).unwrap();
    let index = ClassIndex::default();
    let (aliases, paths) = (BTreeMap::new(), HashSet::new());
    let ctx = Project { index: &index, aliases: &aliases, modules: &crate::modules::Modules::default(), paths: &paths, docs: &docs };
    let unused = unused_registry_entries(&compiled, files[1..].iter(), &ctx);
    assert_eq!(unused.len(), 1);
    assert_eq!((unused[0].path.as_str(), unused[0].line, unused[0].message.as_str()), ("codes.json", 4, "`b` is listed here but no watched call passes it"));
}

// ---- Python

#[test]
fn flags_raise_forms_but_not_bare_reraise() {
    let src = "raise ValueError('x')\nraise ValueError\nraise errors.ValueError() from e\ntry:\n    pass\nexcept Exception:\n    raise\nraise Other()\n";
    let r = rule(Language::Python, &["ValueError"], &[], None);
    assert_eq!(lines("t.py", src, &r), [1, 2, 3]);
}

#[test]
fn flags_method_call_on_chain() {
    let src = "x = session.exec(stmt).one()\ny = session.exec(stmt).first()\nz = one()\n";
    let r = rule(Language::Python, &[], &["*.one"], None);
    assert_eq!(lines("t.py", src, &r), [1]);
}

#[test]
fn flags_catch_of_transitive_subclass() {
    let src = "class ExampleException(Exception): pass\nclass Mid(ExampleException): pass\nclass Leaf(Mid): pass\n\
try:\n    pass\nexcept Leaf as e:\n    pass\ntry:\n    pass\nexcept (KeyError, mod.Mid):\n    pass\ntry:\n    pass\nexcept KeyError:\n    pass\n";
    let r = rule(Language::Python, &[], &[], Some("ExampleException"));
    assert_eq!(lines("t.py", src, &r), [6, 10]);
}

#[test]
fn parses_pep695_class_syntax() {
    let file = parse("t.py", "class DomainEvent[T](BaseModel, ABC):\n    pass\n");
    assert!(!file.has_syntax_errors());
    let index = ClassIndex::build(std::slice::from_ref(&file));
    assert!(index.subclasses_of("BaseModel").contains("DomainEvent"));
}

// ---- JavaScript and TypeScript

#[test]
fn js_flags_throw_forms() {
    let src = "throw new HttpError('x');\nthrow new errors.HttpError('x');\nthrow HttpError('x');\nthrow err;\nthrow new Other();\n";
    let r = rule(Language::JavaScript, &["HttpError"], &[], None);
    assert_eq!(lines("t.js", src, &r), [1, 2, 3]);
}

#[test]
fn js_flags_calls_and_constructors() {
    let src = "console.log(1);\nconst t = Date.now();\nconst d = new Date();\nlog(1);\nMath.random();\n";
    let r = rule(Language::JavaScript, &[], &["console.*", "Date.now", "Date", "Math.random"], None);
    assert_eq!(lines("t.js", src, &r), [1, 2, 3, 5]);
}

#[test]
fn js_sees_through_chains_and_wrappers() {
    let src = "db.query(sql).one();\nnew Date().getTime();\n(x as any).save();\ny!.save();\n(z).save();\n";
    let r = rule(Language::TypeScript, &[], &["*.one", "new Date().getTime", "x.save", "y.save", "z.save"], None);
    assert_eq!(lines("t.ts", src, &r), [1, 2, 3, 4, 5]);
}

#[test]
fn ts_parses_types_generics_and_decorators() {
    let src = "@Injectable()\nclass A<T extends object> implements I {\n  constructor(private readonly s: Svc) {}\n  run(x: T): Promise<void> { throw new BadRequestException('x'); }\n}\ninterface I { run(x: unknown): void }\n";
    let file = parse("a.ts", src);
    assert!(!file.has_syntax_errors());
    let r = rule(Language::TypeScript, &["BadRequestException"], &[], None);
    assert_eq!(lines("a.ts", src, &r), [4]);
}

#[test]
fn tsx_and_jsx_parse() {
    let tsx = "const A = (p: { n: number }) => <div onClick={() => track('a')}>{p.n}</div>;\n";
    let jsx = "const A = (p) => <div onClick={() => track('a')}>{p.n}</div>;\n";
    assert!(!parse("a.tsx", tsx).has_syntax_errors());
    assert!(!parse("a.jsx", jsx).has_syntax_errors());
    let r = rule(Language::TypeScript, &[], &["track"], None);
    assert_eq!(lines("a.tsx", tsx, &r), [1]);
}

#[test]
fn extensions_map_to_languages() {
    for (path, language) in [
        ("a.py", Language::Python),
        ("a.js", Language::JavaScript),
        ("a.mjs", Language::JavaScript),
        ("a.cjs", Language::JavaScript),
        ("a.jsx", Language::JavaScript),
        ("a.ts", Language::TypeScript),
        ("a.tsx", Language::TypeScript),
        ("a.mts", Language::TypeScript),
        ("a.rs", Language::Rust),
    ] {
        assert_eq!(parse(path, "").language, language, "{path}");
    }
}

// ---- imports

fn ts_rule() -> Rule {
    Rule { language: vec![Language::TypeScript], ..empty_rule() }
}

fn aliases() -> BTreeMap<String, String> {
    BTreeMap::from([("@/".to_string(), "src/".to_string())])
}

fn project(paths: &[&str]) -> HashSet<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

#[test]
fn every_import_form_is_found() {
    let src = "import a from 'pkg';\nimport 'other';\nimport type { T } from 'pkg';\nexport * from 'pkg';\n\
export { x } from 'pkg';\nconst m = await import('pkg');\nconst r = require('pkg');\nimport e = require('pkg');\n";
    let r = Rule { forbid_import: vec!["pkg".into()], ..ts_rule() };
    assert_eq!(lines("t.ts", src, &r), [1, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn relative_to_steps_out_of_the_base_directory() {
    assert_eq!(relative_to("", "src/a.ts"), "src/a.ts");
    assert_eq!(relative_to("web", "web/src/a.ts"), "src/a.ts");
    assert_eq!(relative_to("web/app", "web/lib/a.ts"), "../lib/a.ts");
    assert_eq!(relative_to("web", "api/a.ts"), "../api/a.ts");
}

#[test]
fn imports_resolve_aliases_relative_paths_and_extensions() {
    let paths = project(&[
        "src/components/ui/button.tsx",
        "src/hooks/use-mobile.ts",
        "src/parties/hooks/useParty.tsx",
        "src/utils/index.ts",
    ]);
    let src = "import { B } from '@/components/ui/button';\nimport u from '@/hooks/use-mobile';\n\
import p from '../../parties/hooks/useParty';\nimport x from '@/utils';\nimport R from 'react';\n";
    let r = Rule {
        forbid_import: vec!["src/**".into()],
        except_import: vec!["src/components/ui/**".into(), "src/utils/**".into()],
        ..ts_rule()
    };
    let found = hits_with("src/components/ui/sidebar.tsx", src, &r, &aliases(), &paths);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 3]);
    assert!(found[0].1.contains("src/hooks/use-mobile.ts"), "{found:?}");
    assert!(found[1].1.contains("src/parties/hooks/useParty.tsx"), "{found:?}");
}

#[test]
fn package_patterns_match_the_specifier_as_written() {
    let src = "import a from '@base-ui/react';\nimport b from '@base-ui/react/dialog';\nimport c from '@base-ui/reactish';\n";
    let r = Rule { forbid_import: vec!["@base-ui/react".into(), "@base-ui/react/**".into()], ..ts_rule() };
    assert_eq!(lines("t.ts", src, &r), [1, 2]);
}

#[test]
fn imports_that_leave_the_project_are_left_alone() {
    let src = "import a from '../../../outside';\n";
    let r = Rule { forbid_import: vec!["**".into()], ..ts_rule() };
    // Still matched as the raw specifier, but never resolved to a bogus project path.
    assert_eq!(hits_with("src/a.ts", src, &r, &aliases(), &HashSet::new())[0].1, "importing `../../../outside` is forbidden");
}

// ---- JSX elements

#[test]
fn jsx_elements_are_matched_by_name() {
    let src = "const a = <button onClick={f}>x</button>;\nconst b = <input />;\nconst c = <Button />;\nconst d = <table.Row />;\n";
    let r = Rule { forbid_element: vec!["button".into(), "input".into(), "table".into()], ..ts_rule() };
    assert_eq!(lines("t.tsx", src, &r), [1, 2]);
}

// ---- object properties

#[test]
fn forbid_property_sees_object_keys_but_not_jsx_attributes() {
    let src = "useThing({ onError: () => {}, onSuccess });\nconst o = { onError() {}, 'onError': 1, onError };\n\
const p = <img onError={x} />;\nconst { onError: renamed } = o;\n";
    let r = Rule { forbid_property: vec!["onError".into()], ..ts_rule() };
    assert_eq!(lines("t.tsx", src, &r), [1, 2, 2, 2]);
}

#[test]
fn within_property_only_counts_hits_inside_that_property() {
    let src = "mutate(v, {\n  onError: () => toast.add({ a: 1 }),\n  onSuccess: () => toast.add({ b: 1 }),\n});\ntoast.add({ c: 1 });\n";
    let r = Rule { forbid_call: vec!["toast.add".into()], within_property: vec!["onError".into()], ..ts_rule() };
    assert_eq!(lines("t.ts", src, &r), [2]);
}

fn literal_rule() -> Rule {
    Rule {
        property_value: Some(crate::config::PropertyValue {
            names: vec!["title".into(), "description".into()],
            in_call: vec!["toast.add".into()],
            forbid_literal: true,
            array_starts_with: None,
            forbid_value: Vec::new(),
            require_value: Vec::new(),
        }),
        ..ts_rule()
    }
}

#[test]
fn forbid_literal_flags_text_that_skips_translation() {
    let src = "toast.add({ title: 'Hi', description: `plain`, type: 'error' });\n\
toast.add({ title: t('a'), description: `${n} files` });\nother({ title: 'Hi' });\nconst x = { title: 'Hi' };\n";
    assert_eq!(lines("t.tsx", src, &literal_rule()), [1, 1]);
}

#[test]
fn array_starts_with_checks_literal_arrays_only() {
    let src = "useQuery({ queryKey: [accountID, 'a'] });\nuseQuery({ queryKey: ['x', accountID] });\n\
useQuery({ queryKey: key(accountID) });\nuseQuery({ queryKey: [] });\nuseQuery({ queryKey: [accountID] as const });\n\
useQuery({ queryKey: [...base, accountID] });\n";
    let r = Rule {
        property_value: Some(crate::config::PropertyValue {
            names: vec!["queryKey".into()],
            in_call: Vec::new(),
            forbid_literal: false,
            array_starts_with: Some("accountID".into()),
            forbid_value: Vec::new(),
            require_value: Vec::new(),
        }),
        ..ts_rule()
    };
    let found = hits("t.ts", src, &r);
    assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 4, 6]);
    assert!(found[1].1.contains("an empty array"), "{found:?}");
}

// ---- Rust

fn rs_rule() -> Rule {
    Rule { language: vec![Language::Rust], ..empty_rule() }
}

#[test]
fn rust_calls_cover_functions_methods_and_macros() {
    let src = "fn f() {\n    let a = Vec::new();\n    let b = x.unwrap();\n    let c = s.parse::<i32>().unwrap();\n\
let d = foo()?.bar();\n    println!(\"hi\");\n    std::fs::read_to_string(\"p\").expect(\"e\");\n    panic!(\"no\");\n    (a.b)();\n    Vec::<i32>::new();\n}\n";
    let r = Rule {
        forbid_call: vec!["Vec::new".into(), "*.unwrap".into(), "*.bar".into(), "println!".into(), "std::fs::*".into(), "*.expect".into(), "panic!".into(), "a.b".into()],
        ..rs_rule()
    };
    // Vec::<i32>::new() is Vec::new too (generics are looked through).
    assert_eq!(lines("a.rs", src, &r), [2, 3, 4, 5, 6, 7, 7, 8, 9, 10]);
}

#[test]
fn rust_does_not_see_calls_inside_macro_arguments() {
    // Tree-sitter keeps a macro's arguments as flat tokens, so this unwrap is invisible.
    let src = "fn f() { assert_eq!(x.unwrap(), 1); }\n";
    let r = Rule { forbid_call: vec!["*.unwrap".into(), "assert_eq!".into()], ..rs_rule() };
    assert_eq!(hits("a.rs", src, &r).len(), 1);
}

// ---- Go

fn go_rule() -> Rule {
    Rule { language: vec![Language::Go], ..empty_rule() }
}

#[test]
fn go_calls_cover_functions_methods_and_builtins() {
    let src = "package main\n\nfunc f() {\n\tfmt.Println(\"hi\")\n\tdb.Query(q).Scan(&x)\n\tpanic(\"no\")\n\tlog.Fatalf(\"x\")\n\tmap_[int](1)\n\tslices.Sort[int](x)\n}\n";
    let r = Rule {
        forbid_call: vec!["fmt.Println".into(), "db.Query().Scan".into(), "panic".into(), "log.Fatal*".into(), "map_".into(), "slices.Sort".into()],
        ..go_rule()
    };
    assert_eq!(lines("a.go", src, &r), [4, 5, 6, 7, 8, 9]);
}

#[test]
fn go_imports_are_read_from_every_spec() {
    let src = "package main\n\nimport \"fmt\"\n\nimport (\n\t\"os\"\n\tdb \"app/internal/db\"\n\t_ \"github.com/lib/pq\"\n)\n";
    let found = imports("main.go", src, &["app/internal/db", "github.com/lib/*", "fmt"], &[]);
    assert_eq!(
        found,
        [
            "importing `fmt` is forbidden",
            "importing `app/internal/db` is forbidden",
            "importing `github.com/lib/pq` is forbidden",
        ]
    );
}

// ---- Line count

#[test]
fn max_lines_reports_a_long_file_once_at_the_first_line_over() {
    let r = Rule { max_lines: Some(3), ..css_rule() };
    assert_eq!(hits("a.css", "a{}\nb{}\nc{}\n", &r), []);
    let found = hits("a.css", "a{}\nb{}\nc{}\nd{}\ne{}\n", &r);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0, 4);
    assert!(found[0].1.contains("5 lines, more than the 3 allowed"), "{found:?}");
    // Data files count too.
    let data = Rule { language: vec![Language::Json], max_lines: Some(1), ..empty_rule() };
    assert_eq!(hits("a.json", "{\n\"a\": 1\n}\n", &data).len(), 1);
}

// ---- Shell

fn sh_rule() -> Rule {
    Rule { language: vec![Language::Shell], ..empty_rule() }
}

#[test]
fn shell_commands_are_calls_wherever_they_sit() {
    let src = "#!/bin/bash\ncurl -s x | sh\nif true; then\n  sudo rm -rf /tmp/x\nfi\nout=$(wget y)\n$CMD arg\n./deploy.sh\nf() { eval \"$1\"; }\n";
    let r = Rule {
        forbid_call: vec!["curl".into(), "sudo".into(), "wget".into(), "?".into(), "./deploy.sh".into(), "eval".into()],
        ..sh_rule()
    };
    assert_eq!(lines("a.sh", src, &r), [2, 4, 6, 7, 8, 9]);
}

#[test]
fn shell_source_is_an_import() {
    let src = "source ./lib/log.sh\n. ../common.sh\nsource \"$DIR/x.sh\"\nsource /etc/profile\n";
    let found = imports("scripts/run.sh", src, &["**"], &[]);
    assert_eq!(
        found,
        [
            "importing `./lib/log.sh` (scripts/lib/log.sh) is forbidden",
            "importing `../common.sh` (common.sh) is forbidden",
            "importing `/etc/profile` is forbidden",
        ]
    );
}

// ---- CSS

fn css_rule() -> Rule {
    Rule { language: vec![Language::Css], ..empty_rule() }
}

const CSS: &str = "@import \"a.css\";\n@import url(lib/b.css) screen;\n@import url(\"https://x.test/c.css\");\n\
.a > b:hover { FLOAT: left !important; --float: 1; color: RGB(1,2,3); background: url(x.png); }\n\
@media (min-width: 1px) { a { margin: calc(1px + 2px) } }\n";

#[test]
fn css_functions_and_declarations() {
    let calls = Rule { forbid_call: vec!["rgb".into(), "calc".into()], ..css_rule() };
    assert_eq!(lines("a.css", CSS, &calls), [4, 5]);
    let props = Rule { forbid_property: vec!["float".into()], ..css_rule() };
    assert_eq!(lines("a.css", CSS, &props), [4]);
}

#[test]
fn css_imports_are_resolved_against_the_stylesheet() {
    let found = imports("styles/app.css", CSS, &["**"], &[]);
    assert_eq!(
        found,
        [
            "importing `a.css` (styles/a.css) is forbidden",
            "importing `lib/b.css` (styles/lib/b.css) is forbidden",
            "importing `https://x.test/c.css` is forbidden",
        ]
    );
}

// ---- HTML

fn html_rule() -> Rule {
    Rule { language: vec![Language::Html], ..empty_rule() }
}

#[test]
fn html_elements_match_case_insensitively_including_self_closing() {
    let src = "<div>\n<BUTTON class=\"a\">x</BUTTON>\n<img src=\"i.png\">\n<input />\n<script>var a = 1;</script>\n</div>\n";
    let r = Rule { forbid_element: vec!["button".into(), "input".into(), "script".into(), "img".into()], ..html_rule() };
    assert_eq!(lines("a.html", src, &r), [2, 3, 4, 5]);
}

#[test]
fn html_attributes_are_properties() {
    let src = "<p onclick=\"go()\" STYLE='x'>\n<input disabled data-id=\"7\">\n</p>\n";
    let r = Rule { forbid_property: ["onclick", "style", "disabled"].into_iter().map(String::from).collect(), ..html_rule() };
    assert_eq!(lines("a.html", src, &r), [1, 1, 2]);
}

/// Import messages of `source` with the project's files and `go.mod` modules known.
fn imports_in(path: &str, source: &str, files: &[&str], go: &[(&str, &str)], forbid: &[&str]) -> Vec<String> {
    let file = parse(path, source);
    let index = ClassIndex::build(std::slice::from_ref(&file));
    let r = Rule { forbid_import: forbid.iter().map(|s| s.to_string()).collect(), ..empty_rule() };
    let compiled = CompiledRule::new("r", &r).unwrap();
    let paths = project(files);
    let list: Vec<String> = files.iter().map(|f| f.to_string()).collect();
    let modules = crate::modules::Modules {
        go: crate::modules::GoModules::for_test(go),
        py: crate::modules::PythonRoots::load(&BTreeMap::new(), &list),
        ..Default::default()
    };
    let docs = BTreeMap::new();
    let aliases = BTreeMap::new();
    let ctx = Project { index: &index, aliases: &aliases, modules: &modules, paths: &paths, docs: &docs };
    let mut out = Vec::new();
    check_file(&file, &compiled, &ctx, &mut out);
    out.into_iter().map(|v| v.message).collect()
}

#[test]
fn go_imports_inside_the_module_resolve_to_project_directories() {
    let src = "package main\n\nimport (\n\t\"fmt\"\n\t\"github.com/acme/app/internal/db\"\n\t\"github.com/lib/pq\"\n)\n";
    let found = imports_in("cmd/main.go", src, &[], &[("github.com/acme/app", "")], &["internal/db", "internal/db/**", "github.com/lib/**"]);
    assert_eq!(
        found,
        [
            "importing `github.com/acme/app/internal/db` (internal/db) is forbidden",
            "importing `github.com/lib/pq` is forbidden",
        ]
    );
}

#[test]
fn python_imports_resolve_through_source_roots_and_relative_dots() {
    let files = ["src/app/__init__.py", "src/app/db/__init__.py", "src/app/db/query.py", "src/app/ui/page.py", "src/app/ui/helpers.py"];
    let src = "import os\nimport app.db.query as q\nfrom app.db import query, connect\nfrom . import helpers\nfrom ..db import *\nfrom sqlalchemy import select\n";
    let found = imports_in("src/app/ui/page.py", src, &files, &[], &["src/app/db/**", "src/app/db", "sqlalchemy.**"]);
    assert_eq!(
        found,
        [
            "importing `app.db.query` (src/app/db/query.py) is forbidden",
            "importing `app.db.query` (src/app/db/query.py) is forbidden",
            "importing `app.db.connect` (src/app/db/__init__.py) is forbidden",
            "importing `..db.*` (src/app/db/__init__.py) is forbidden",
            "importing `sqlalchemy.select` is forbidden",
        ]
    );
}

#[test]
fn python_patterns_may_use_dotted_module_names() {
    let found = imports_in("a.py", "import app.db.query\nfrom app.db import x\nimport app.ui\n", &[], &[], &["app.db.**"]);
    assert_eq!(found.len(), 2, "{found:?}");
}

fn imports(path: &str, source: &str, forbid: &[&str], except: &[&str]) -> Vec<String> {
    let r = Rule {
        forbid_import: forbid.iter().map(|s| s.to_string()).collect(),
        except_import: except.iter().map(|s| s.to_string()).collect(),
        ..rs_rule()
    };
    hits(path, source, &r).into_iter().map(|h| h.1).collect()
}

#[test]
fn rust_use_declarations_are_expanded() {
    let src = "use std::collections::{HashMap, hash_map::Entry, self};\nuse a::b as c;\npub use d::e::F;\nuse g::*;\nextern crate h;\nuse {i::j, k};\n";
    let all = imports("src/lib.rs", src, &["**"], &[]);
    assert_eq!(
        all,
        [
            "importing `std::collections::HashMap` is forbidden",
            "importing `std::collections::hash_map::Entry` is forbidden",
            "importing `std::collections` is forbidden",
            "importing `a::b` is forbidden",
            "importing `d::e::F` is forbidden",
            "importing `g::*` is forbidden",
            "importing `h` is forbidden",
            "importing `i::j` is forbidden",
            "importing `k` is forbidden",
        ]
    );
}

#[test]
fn rust_patterns_are_written_with_double_colons() {
    let src = "use sqlx::PgPool;\nuse sqlx;\nuse crate::db::Pool;\nuse crate::api::Router;\nuse std::fmt;\n";
    let found = imports("src/domain/user.rs", src, &["sqlx", "sqlx::**", "crate::db::**"], &[]);
    assert_eq!(
        found,
        [
            "importing `sqlx::PgPool` is forbidden",
            "importing `sqlx` is forbidden",
            "importing `crate::db::Pool` is forbidden",
        ]
    );
}

#[test]
fn rust_self_and_super_resolve_to_the_module_path() {
    // src/api/handlers.rs is the module crate::api::handlers.
    let src = "use super::db::Pool;\nuse self::helpers::h;\nuse super::super::config::Settings;\nuse crate::db::Pool as P;\n";
    let found = imports("crate/src/api/handlers.rs", src, &["crate::api::**", "crate::config::**"], &[]);
    assert_eq!(
        found,
        [
            "importing `super::db::Pool` (crate::api::db::Pool) is forbidden",
            "importing `self::helpers::h` (crate::api::handlers::helpers::h) is forbidden",
            "importing `super::super::config::Settings` (crate::config::Settings) is forbidden",
        ]
    );
    // mod.rs and lib.rs are their directory's module, and the crate root.
    let found = imports("src/api/mod.rs", "use self::x::Y;\n", &["crate::api::x::**"], &[]);
    assert_eq!(found, ["importing `self::x::Y` (crate::api::x::Y) is forbidden"]);
    let found = imports("src/lib.rs", "use self::x::Y;\n", &["crate::x::**"], &[]);
    assert_eq!(found, ["importing `self::x::Y` (crate::x::Y) is forbidden"]);
}

#[test]
fn rust_inline_modules_are_part_of_the_module_path() {
    let src = "use super::*;\n#[cfg(test)]\nmod tests {\n    use super::*;\n    use super::helper;\n}\n";
    let found = imports("src/a.rs", src, &["crate::**"], &[]);
    assert_eq!(
        found,
        [
            "importing `super::*` (crate::*) is forbidden", // a top-level module's parent is the crate root
            "importing `super::*` (crate::a::*) is forbidden",
            "importing `super::helper` (crate::a::helper) is forbidden",
        ]
    );
}

#[test]
fn rust_super_above_the_crate_root_is_left_as_written() {
    let found = imports("src/lib.rs", "use super::x;\n", &["super::**"], &[]);
    assert_eq!(found, ["importing `super::x` is forbidden"]);
}

#[test]
fn rust_import_exceptions() {
    let src = "use crate::domain::User;\nuse crate::infra::Db;\nuse std::fmt;\n";
    let found = imports("src/domain/user.rs", src, &["crate::**"], &["crate::domain::**"]);
    assert_eq!(found, ["importing `crate::infra::Db` is forbidden"]);
}

// ---- file names

#[test]
fn filename_rule_matches_on_the_base_name_only() {
    let rule = Rule { require_filename: vec!["test_*.py".into(), "conftest.py".into()], ..empty_rule() };
    let compiled = CompiledRule::new("r", &rule).unwrap();
    assert!(check_filename("tests/test_users.py", &compiled).is_none());
    assert!(check_filename("tests/unit/conftest.py", &compiled).is_none());
    let hit = check_filename("tests/helpers.py", &compiled).unwrap();
    assert!(hit.message.contains("`helpers.py`"));
    assert!(check_filename("tests/data.json", &compiled).is_some());
    assert!(check_filename("tests/mytest_x.py", &compiled).is_some());
}
