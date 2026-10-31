use super::model::{Doc, dotted};
use super::path::PathPattern;
use super::placeholders::placeholders;
use crate::config::Use;
use crate::source::{Parsers, SourceFile};

fn doc(path: &str, text: &str) -> Doc {
    let (language, tree) = Parsers::new().unwrap().parse(path, text).unwrap();
    Doc::build(&SourceFile { path: path.into(), language, text: text.into(), tree })
}

/// Every node as `path = value` (or `path` for containers), in document order.
fn dump(doc: &Doc) -> Vec<String> {
    let mut out = Vec::new();
    doc.walk(&mut |path, _, node| {
        if path.is_empty() {
            return;
        }
        out.push(match node.scalar() {
            Some(s) => format!("{} = {}", path.join("."), s.text),
            None => path.join("."),
        });
    });
    out
}

#[test]
fn json_values_and_positions() {
    let d = doc("a.json", "{\n  \"a\": {\"b\": [1, \"x\\ny\", true, null]},\n  \"k\": -1.5\n}\n");
    assert_eq!(dump(&d), ["a", "a.b", "a.b.0 = 1", "a.b.1 = x\ny", "a.b.2 = true", "a.b.3 = null", "k = -1.5"]);
    let mut at_k = None;
    d.walk(&mut |path, at, _| {
        if path == ["k"] {
            at_k = Some(at);
        }
    });
    assert_eq!(at_k, Some((3, 3)));
}

#[test]
fn json_comments_are_tolerated() {
    assert_eq!(dump(&doc("a.json", "// note\n{\"a\": 1}\n")), ["a = 1"]);
}

#[test]
fn yaml_blocks_flows_scalars_and_documents() {
    let text = "a: &x\n  b: [1, \"two\", 'it''s']\n  c: {d: e}\nlist:\n  - name: x\n    run: |\n      echo hi\n      echo\n  - plain\nref: *x\nempty:\n---\nb: 1\n";
    let d = doc("a.yaml", text);
    assert_eq!(d.roots.len(), 2);
    assert_eq!(
        dump(&d),
        [
            "a", "a.b", "a.b.0 = 1", "a.b.1 = two", "a.b.2 = it's", "a.c", "a.c.d = e", "list", "list.0", "list.0.name = x",
            "list.0.run = echo hi\necho", "list.1 = plain", "ref = *x", "empty = null", "b = 1"
        ]
    );
}

#[test]
fn toml_quoted_keys_in_headers_and_duplicate_keys() {
    let d = doc("a.toml", "[\"a.b\".c]\nx = 1\n['single']\ny = 2\n[t]\nk = 1\nk = 2\n[[arr]]\n[[arr]]\nv = 3\n");
    assert_eq!(dump(&d), ["a.b", "a.b.c", "a.b.c.x = 1", "single", "single.y = 2", "t", "t.k = 1", "t.k = 2", "arr", "arr.0", "arr.1", "arr.1.v = 3"]);
    // A quoted segment in a path pattern finds the dotted key.
    let found = d.registry(&PathPattern::parse("\"a.b\".c.x").unwrap(), Use::Value);
    assert_eq!(found.into_keys().collect::<Vec<_>>(), ["1"]);
}

#[test]
fn json_duplicate_keys_are_kept() {
    assert_eq!(dump(&doc("a.json", "{\"a\": 1, \"a\": 2}")), ["a = 1", "a = 2"]);
}

#[test]
fn empty_files_have_no_roots() {
    assert!(doc("a.json", "").roots.is_empty());
    assert!(doc("a.yaml", "").roots.is_empty());
    assert_eq!(doc("a.toml", "").roots.len(), 1);
}

fn matches(pattern: &str, path: &str) -> bool {
    let path: Vec<String> = path.split('.').map(String::from).collect();
    PathPattern::parse(pattern).unwrap().matches(&path)
}

#[test]
fn path_patterns() {
    assert!(matches("scripts.postinstall", "scripts.postinstall"));
    assert!(!matches("scripts.postinstall", "x.scripts.postinstall"));
    assert!(matches("jobs.*.runs-on", "jobs.build.runs-on"));
    assert!(!matches("jobs.*.runs-on", "jobs.build.steps.runs-on"));
    assert!(matches("jobs.*.steps.*.uses", "jobs.build.steps.0.uses"));
    assert!(matches("**.version", "version"));
    assert!(matches("**.version", "a.b.version"));
    assert!(!matches("**.version", "a.b.versions"));
    assert!(matches("dependencies.@types/*", "dependencies.@types/node"));
    assert!(matches("dependencies.*", "dependencies.react"));
    assert!(!matches("dependencies.*", "dependencies"));
    assert!(PathPattern::parse("\"a.b\".c").unwrap().matches(&["a.b".into(), "c".into()]));
    assert!(PathPattern::parse("a..b").is_err());
    assert!(PathPattern::parse("\"a.b").is_err());
}

#[test]
fn split_last_names_the_container_and_the_key() {
    let (parent, key) = PathPattern::parse("jobs.*.timeout-minutes").unwrap().split_last().unwrap();
    assert_eq!(key, "timeout-minutes");
    assert!(parent.matches(&["jobs".into(), "build".into()]));
    assert!(!parent.is_literal());
    assert!(PathPattern::parse("jobs.*").unwrap().split_last().is_none());
}

#[test]
fn placeholders_in_messages() {
    let found = |t: &str| placeholders(t).into_iter().collect::<Vec<_>>();
    assert_eq!(found("Hello {{name}}, you have {{ count }}"), ["count", "name"]);
    assert_eq!(found("{count, plural, one {# item} other {# items}} for {user}"), ["count", "user"]);
    assert_eq!(found("No placeholders, just { some braces } and {}"), Vec::<String>::new());
    assert_eq!(found("${version}"), ["version"]);
}

#[test]
fn registries_and_leaves() {
    let d = doc("a.json", "{\"codes\": {\"a_failed.x\": 1, \"b_failed.y\": 2}, \"list\": [\"p\", \"q\"]}");
    let names = |use_| d.registry(&PathPattern::parse("codes.*").unwrap(), use_).into_keys().collect::<Vec<_>>();
    assert_eq!(names(Use::Key), ["a_failed.x", "b_failed.y"]);
    assert_eq!(names(Use::Value), ["1", "2"]);
    assert_eq!(names(Use::Path), ["codes.a_failed.x", "codes.b_failed.y"]);
    let values = d.registry(&PathPattern::parse("list.*").unwrap(), Use::Value);
    assert_eq!(values.into_keys().collect::<Vec<_>>(), ["p", "q"]);
    let leaves: Vec<String> = d.leaves().keys().map(|k| dotted(k)).collect();
    assert_eq!(leaves, ["codes.\"a_failed.x\"", "codes.\"b_failed.y\"", "list"]);
    assert_eq!(d.deepest_existing(&["codes".into(), "zzz".into(), "q".into()]), (1, 2));
}

#[test]
fn a_dotted_key_is_not_a_nested_key() {
    let flat = doc("a.json", "{\"a.b\": 1}");
    let nested = doc("b.json", "{\"a\": {\"b\": 1}}");
    assert_ne!(flat.leaves().keys().collect::<Vec<_>>(), nested.leaves().keys().collect::<Vec<_>>());
    assert_eq!(dotted(&["a.b".into(), "c".into()]), "\"a.b\".c");
    // The missing key is reported at the deepest key that really exists.
    assert_eq!(nested.deepest_existing(&["a.b".into()]), (1, 1));
}
