use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use tree_sitter::Node;

use crate::adapter::{self, last_segment};
use crate::config::Rule;
use crate::source::SourceFile;

pub struct Violation {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub rule_id: String,
    pub message: String,
}

/// A rule with its patterns compiled, ready to run against files.
pub struct CompiledRule<'a> {
    pub id: &'a str,
    pub rule: &'a Rule,
    raise: GlobSet,
    call: GlobSet,
    catch_root: Option<&'a str>,
    filename: GlobSet,
}

impl<'a> CompiledRule<'a> {
    pub fn new(id: &'a str, rule: &'a Rule) -> Result<Self> {
        Ok(CompiledRule {
            id,
            rule,
            raise: compile(&rule.forbid_raise).with_context(|| format!("rule `{id}` forbid_raise"))?,
            call: compile(&rule.forbid_call).with_context(|| format!("rule `{id}` forbid_call"))?,
            catch_root: rule.forbid_catch.as_ref().map(|c| c.subclass_of.as_str()),
            filename: compile(&rule.require_filename).with_context(|| format!("rule `{id}` require_filename"))?,
        })
    }
}

fn compile(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

/// Class name -> names of its direct bases, across every file in the project.
/// Resolution is by simple name, not by import path.
#[derive(Default)]
pub struct ClassIndex {
    bases: BTreeMap<String, BTreeSet<String>>,
}

impl ClassIndex {
    pub fn build(files: &[SourceFile]) -> ClassIndex {
        let mut index = ClassIndex::default();
        for file in files {
            let adapter = adapter::for_language(file.language);
            if !adapter.indexes_classes() {
                continue;
            }
            visit(file.tree.root_node(), &mut |node| {
                if let Some((name, bases)) = adapter.class_bases(node, &file.text) {
                    index.bases.entry(name).or_default().extend(bases);
                }
            });
        }
        index
    }

    pub fn defines(&self, class: &str) -> bool {
        self.bases.contains_key(class)
    }

    /// `root` and every class that transitively inherits from it.
    pub fn subclasses_of(&self, root: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::from([root.to_string()]);
        loop {
            let before = found.len();
            for (class, bases) in &self.bases {
                if bases.iter().any(|b| found.contains(b)) {
                    found.insert(class.clone());
                }
            }
            if found.len() == before {
                return found;
            }
        }
    }
}

/// A file whose name matches none of the rule's `require_filename` globs.
pub fn check_filename(path: &str, rule: &CompiledRule) -> Option<Violation> {
    if rule.rule.require_filename.is_empty() {
        return None;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    if rule.filename.is_match(name) {
        return None;
    }
    Some(Violation {
        path: path.to_string(),
        line: 1,
        column: 1,
        rule_id: rule.id.to_string(),
        message: format!(
            "file name `{name}` is not allowed here; allowed names: {}",
            rule.rule.require_filename.join(", ")
        ),
    })
}

pub fn check_file(
    file: &SourceFile,
    rule: &CompiledRule,
    index: &ClassIndex,
    out: &mut Vec<Violation>,
) {
    let adapter = adapter::for_language(file.language);
    let catch_classes = rule.catch_root.map(|root| (root, index.subclasses_of(root)));
    let src = file.text.as_str();
    let mut report = |node: Node, message: String| {
        let pos = node.start_position();
        out.push(Violation {
            path: file.path.clone(),
            line: pos.row + 1,
            column: pos.column + 1,
            rule_id: rule.id.to_string(),
            message,
        });
    };

    visit(file.tree.root_node(), &mut |node| {
        if !rule.raise.is_empty()
            && let Some(name) = adapter.raised_name(node, src)
            && (rule.raise.is_match(&name) || rule.raise.is_match(last_segment(&name)))
        {
            report(node, format!("raising `{name}` is forbidden"));
        }
        if !rule.call.is_empty()
            && let Some(name) = adapter.call_name(node, src)
            && rule.call.is_match(&name)
        {
            report(node, format!("calling `{name}` is forbidden"));
        }
        if let Some((root, classes)) = &catch_classes
            && let Some(caught) = adapter.caught_types(node, src)
        {
            for name in caught {
                if classes.contains(last_segment(&name)) {
                    report(node, format!("catching `{name}` ({root} or a subclass) is forbidden"));
                }
            }
        }
    });
}

/// Depth-first walk over `node` and all its descendants.
fn visit(node: Node, f: &mut impl FnMut(Node)) {
    let mut cursor = node.walk();
    loop {
        f(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClassSelector, Language};
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
            require_filename: Vec::new(),
            rationale: String::new(),
            fix: String::new(),
        }
    }

    /// Lines (1-based) of every violation of `rule` in `source`, parsed as the file `path`.
    fn lines(path: &str, source: &str, rule: &Rule) -> Vec<usize> {
        hits(path, source, rule).into_iter().map(|h| h.0).collect()
    }

    fn hits(path: &str, source: &str, rule: &Rule) -> Vec<(usize, String)> {
        let file = parse(path, source);
        let index = ClassIndex::build(std::slice::from_ref(&file));
        let compiled = CompiledRule::new("r", rule).unwrap();
        let mut out = Vec::new();
        check_file(&file, &compiled, &index, &mut out);
        out.into_iter().map(|v| (v.line, v.message)).collect()
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
        ] {
            assert_eq!(parse(path, "").language, language, "{path}");
        }
    }

    // ---- file names

    #[test]
    fn filename_rule_matches_on_the_base_name_only() {
        let rule = Rule {
            language: Vec::new(),
            forbid_raise: Vec::new(),
            forbid_call: Vec::new(),
            forbid_catch: None,
            require_filename: vec!["test_*.py".into(), "conftest.py".into()],
            rationale: String::new(),
            fix: String::new(),
        };
        let compiled = CompiledRule::new("r", &rule).unwrap();
        assert!(check_filename("tests/test_users.py", &compiled).is_none());
        assert!(check_filename("tests/unit/conftest.py", &compiled).is_none());
        let hit = check_filename("tests/helpers.py", &compiled).unwrap();
        assert!(hit.message.contains("`helpers.py`"));
        assert!(check_filename("tests/data.json", &compiled).is_some());
        assert!(check_filename("tests/mytest_x.py", &compiled).is_some());
    }
}
