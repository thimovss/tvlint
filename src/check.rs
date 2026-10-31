use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use tree_sitter::Node;

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
}

impl<'a> CompiledRule<'a> {
    pub fn new(id: &'a str, rule: &'a Rule) -> Result<Self> {
        Ok(CompiledRule {
            id,
            rule,
            raise: compile(&rule.forbid_raise).with_context(|| format!("rule `{id}` forbid_raise"))?,
            call: compile(&rule.forbid_call).with_context(|| format!("rule `{id}` forbid_call"))?,
            catch_root: rule.forbid_catch.as_ref().map(|c| c.subclass_of.as_str()),
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
            visit(file.tree.root_node(), &mut |node| {
                if node.kind() != "class_definition" {
                    return;
                }
                let Some(name) = node.child_by_field_name("name") else { return };
                let entry = index.bases.entry(text(name, &file.text).to_string()).or_default();
                if let Some(supers) = node.child_by_field_name("superclasses") {
                    let mut cursor = supers.walk();
                    for base in supers.named_children(&mut cursor) {
                        if let Some(dotted) = dotted_name(base, &file.text) {
                            entry.insert(last_segment(&dotted).to_string());
                        }
                    }
                }
            });
        }
        index
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

pub fn check_file(
    file: &SourceFile,
    rule: &CompiledRule,
    index: &ClassIndex,
    out: &mut Vec<Violation>,
) {
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

    visit(file.tree.root_node(), &mut |node| match node.kind() {
        "raise_statement" if !rule.raise.is_empty() => {
            if let Some(name) = raised_name(node, src)
                && (rule.raise.is_match(&name) || rule.raise.is_match(last_segment(&name)))
            {
                report(node, format!("raising `{name}` is forbidden"));
            }
        }
        "call" if !rule.call.is_empty() => {
            if let Some(function) = node.child_by_field_name("function") {
                let name = callee_name(function, src);
                if rule.call.is_match(&name) {
                    report(node, format!("calling `{name}` is forbidden"));
                }
            }
        }
        "except_clause" | "except_group_clause" => {
            if let Some((root, classes)) = &catch_classes {
                for caught in caught_types(node, src) {
                    if classes.contains(last_segment(&caught)) {
                        report(node, format!("catching `{caught}` ({root} or a subclass) is forbidden"));
                    }
                }
            }
        }
        _ => {}
    });
}

/// Name of the exception in `raise Foo(...)`, `raise Foo` or `raise mod.Foo(...) from e`.
fn raised_name(raise: Node, src: &str) -> Option<String> {
    let cause = raise.child_by_field_name("cause").map(|n| n.id());
    let mut cursor = raise.walk();
    let expr = raise
        .named_children(&mut cursor)
        .find(|n| n.kind() != "comment" && Some(n.id()) != cause)?;
    let target = if expr.kind() == "call" { expr.child_by_field_name("function")? } else { expr };
    dotted_name(target, src)
}

/// The exception types of an `except` clause, flattening `except (A, B) as e`.
fn caught_types(clause: Node, src: &str) -> Vec<String> {
    let mut cursor = clause.walk();
    let Some(mut first) = clause
        .named_children(&mut cursor)
        .find(|n| !matches!(n.kind(), "block" | "comment"))
    else {
        return Vec::new();
    };
    if first.kind() == "as_pattern" {
        let Some(inner) = first.named_child(0) else { return Vec::new() };
        first = inner;
    }
    let mut names = Vec::new();
    if matches!(first.kind(), "tuple" | "parenthesized_expression" | "expression_list") {
        let mut cursor = first.walk();
        for item in first.named_children(&mut cursor) {
            names.extend(dotted_name(item, src));
        }
    } else {
        names.extend(dotted_name(first, src));
    }
    names
}

/// `a.b.c` for plain identifier/attribute chains, otherwise `None`.
fn dotted_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, src).to_string()),
        "attribute" => {
            let object = dotted_name(node.child_by_field_name("object")?, src)?;
            let attr = text(node.child_by_field_name("attribute")?, src);
            Some(format!("{object}.{attr}"))
        }
        _ => None,
    }
}

/// Like `dotted_name` but total: calls render as `f()` and anything else as `?`,
/// so `session.exec(q).one` becomes `session.exec().one`.
fn callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" => text(node, src).to_string(),
        "attribute" => {
            let object = node
                .child_by_field_name("object")
                .map_or_else(|| "?".to_string(), |o| callee_name(o, src));
            let attr = node.child_by_field_name("attribute").map_or("?", |a| text(a, src));
            format!("{object}.{attr}")
        }
        "call" => {
            let function = node
                .child_by_field_name("function")
                .map_or_else(|| "?".to_string(), |f| callee_name(f, src));
            format!("{function}()")
        }
        _ => "?".to_string(),
    }
}

fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    &src[node.byte_range()]
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
    use tree_sitter::Parser;

    fn parse(text: &str) -> SourceFile {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_python::LANGUAGE.into()).unwrap();
        SourceFile {
            path: "t.py".into(),
            language: Language::Python,
            text: text.into(),
            tree: parser.parse(text, None).unwrap(),
        }
    }

    fn rule(raise: &[&str], call: &[&str], catch: Option<&str>) -> Rule {
        Rule {
            language: Language::Python,
            forbid_raise: raise.iter().map(|s| s.to_string()).collect(),
            forbid_call: call.iter().map(|s| s.to_string()).collect(),
            forbid_catch: catch.map(|c| ClassSelector { subclass_of: c.into() }),
            rationale: String::new(),
            fix: String::new(),
        }
    }

    fn run(source: &str, rule: &Rule) -> Vec<(usize, String)> {
        let file = parse(source);
        let index = ClassIndex::build(std::slice::from_ref(&file));
        let compiled = CompiledRule::new("r", rule).unwrap();
        let mut out = Vec::new();
        check_file(&file, &compiled, &index, &mut out);
        out.into_iter().map(|v| (v.line, v.message)).collect()
    }

    #[test]
    fn flags_raise_forms_but_not_bare_reraise() {
        let src = "raise ValueError('x')\nraise ValueError\nraise errors.ValueError() from e\ntry:\n    pass\nexcept Exception:\n    raise\nraise Other()\n";
        let hits = run(src, &rule(&["ValueError"], &[], None));
        assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn flags_method_call_on_chain() {
        let src = "x = session.exec(stmt).one()\ny = session.exec(stmt).first()\nz = one()\n";
        let hits = run(src, &rule(&[], &["*.one"], None));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 1);
    }

    #[test]
    fn flags_catch_of_transitive_subclass() {
        let src = "class ExampleException(Exception): pass\nclass Mid(ExampleException): pass\nclass Leaf(Mid): pass\n\
try:\n    pass\nexcept Leaf as e:\n    pass\ntry:\n    pass\nexcept (KeyError, mod.Mid):\n    pass\ntry:\n    pass\nexcept KeyError:\n    pass\n";
        let hits = run(src, &rule(&[], &[], Some("ExampleException")));
        assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), [6, 10]);
    }

    #[test]
    fn parses_pep695_class_syntax() {
        let file = parse("class DomainEvent[T](BaseModel, ABC):\n    pass\n");
        assert!(!file.has_syntax_errors());
        let index = ClassIndex::build(std::slice::from_ref(&file));
        assert!(index.subclasses_of("BaseModel").contains("DomainEvent"));
    }
}
