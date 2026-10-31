//! Checks of source code, run against the syntax tree through an adapter.

use std::collections::HashSet;

use globset::GlobSet;
use tree_sitter::Node;

use crate::adapters::{self, Adapter, last_segment};
use crate::config::fill_placeholders;
use crate::source::SourceFile;
use super::compile::{compile_paths, relative_to};
use super::structure;
use super::data_checks::check_data;
use super::{CompiledLiteral, CompiledRule, Literal, Project, Violation};

/// Whether `node` sits inside an object-literal property named by the rule's `within_property`.
pub(super) fn within_property(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str) -> bool {
    if rule.within.is_empty() {
        return true;
    }
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if adapter.object_property(ancestor, src).is_some_and(|p| rule.within.contains(p.key.as_str())) {
            return true;
        }
        current = ancestor.parent();
    }
    false
}

#[cfg(test)]
pub fn check_file(file: &SourceFile, rule: &CompiledRule, ctx: &Project, out: &mut Vec<Violation>) {
    check_file_in_scope(file, rule, ctx, &[], out);
}

/// Like `check_file`, with what the scope's `include` pattern captured, for the `{n}` placeholders in import patterns.
pub fn check_file_in_scope(file: &SourceFile, rule: &CompiledRule, ctx: &Project, captures: &[String], out: &mut Vec<Violation>) {
    if let Some(max) = rule.rule.max_lines {
        let lines = file.text.lines().count();
        if lines > max {
            out.push(Violation {
                path: file.path.clone(),
                line: max + 1,
                column: 1,
                rule_id: rule.id.to_string(),
                message: format!("file has {lines} lines, more than the {max} allowed; the excess starts here"),
            });
        }
    }
    if file.language.is_data() {
        check_data(file, rule, ctx, out);
        return;
    }
    let adapter = adapters::for_language(file.language);
    let catch_classes = rule.catch_root.map(|root| (root, ctx.index.subclasses_of("python", root)));
    let src = file.text.as_str();
    // Import patterns with placeholders are compiled for this file's captures.
    let filled = rule.templated.then(|| {
        let fill = |patterns: &[String]| compile_paths(&patterns.iter().map(|p| fill_placeholders(p, captures)).collect::<Vec<_>>()).unwrap_or_default();
        (fill(&rule.rule.forbid_import), fill(&rule.rule.except_import))
    });
    let (import_set, except_set) = filled.as_ref().map_or((&rule.import, &rule.except_import), |(forbid, except)| (forbid, except));
    let class_scope = structure::class_scope(rule, adapter, ctx);
    let file_wide_call = !rule.require_call.is_empty() && rule.within_function.is_empty();
    let mut called = false;
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
        structure::check_class(rule, adapter, node, src, class_scope.as_ref(), &mut report);
        structure::check_function(rule, adapter, node, src, &mut report);
        if file_wide_call && !called {
            called = adapter.call_name(node, src).is_some_and(|callee| rule.require_call.is_match(&callee));
        }
        if !rule.raise.is_empty()
            && let Some(name) = adapter.raised_name(node, src)
            && (rule.raise.is_match(&name) || rule.raise.is_match(last_segment(&name)))
            && within_property(rule, adapter, node, src)
            && structure::within_function(rule, adapter, node, src)
        {
            report(node, format!("raising `{name}` is forbidden"));
        }
        if !rule.call.is_empty()
            && let Some(name) = adapter.call_name(node, src)
            && rule.call.is_match(&name)
            && within_property(rule, adapter, node, src)
            && structure::within_function(rule, adapter, node, src)
        {
            report(node, format!("calling `{name}` is forbidden"));
        }
        if !import_set.is_empty() {
            // Rust writes paths with `::`; matching treats them as `/` like every other path.
            let key = |s: &str| s.replace("::", "/");
            for import in adapter.imports(node, src, &file.path, ctx) {
                // A package name stays as written; a project path is relative to the rule's config.
                let resolved = if import.resolved == import.specifier { key(&import.resolved) } else { relative_to(&rule.base, &key(&import.resolved)) };
                let hits = |set: &GlobSet| set.is_match(key(&import.specifier)) || set.is_match(&resolved);
                if hits(import_set) && !hits(except_set) {
                    let target = if import.resolved == import.specifier {
                        String::new()
                    } else {
                        format!(" ({})", import.resolved)
                    };
                    report(node, format!("importing `{}`{target} is forbidden", import.specifier));
                }
            }
        }
        if !rule.element.is_empty()
            && let Some(name) = adapter.element_name(node, src)
            && rule.element.is_match(&name)
            && within_property(rule, adapter, node, src)
        {
            report(node, format!("using the `<{name}>` element is forbidden"));
        }
        if let Some(property) = adapter.object_property(node, src) {
            if rule.property.contains(property.key.as_str()) {
                report(node, format!("the `{}` property is forbidden", property.key));
            }
            if let Some(pv) = &rule.property_value
                && pv.names.contains(property.key.as_str())
                && let Some(value) = property.value
                && (pv.in_call.is_empty()
                    || node
                        .parent()
                        .and_then(|object| adapter.argument_of_call(object, src))
                        .is_some_and(|call| pv.in_call.is_match(&call)))
            {
                if pv.forbid_literal && adapter.is_string_literal(value, src) {
                    report(value, format!("`{}` must not be a string literal", property.key));
                }
                if let Some(start) = &pv.starts_with
                    && let Some(elements) = adapter.array_elements(value)
                {
                    let first = elements.first().map_or("", |e| &src[e.byte_range()]);
                    if !start.is_match(first) {
                        let shown = if first.is_empty() { "an empty array" } else { first };
                        report(value, format!("`{}` must start with `{}`, found {shown}", property.key, start.glob().glob()));
                    }
                }
            }
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

    if file_wide_call && !called {
        out.push(Violation {
            path: file.path.clone(),
            line: 1,
            column: 1,
            rule_id: rule.id.to_string(),
            message: format!("the file never calls {}", rule.rule.require_call.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ")),
        });
    }
    if let Some(literal) = &rule.literal {
        let entries = literal.entries(ctx);
        for found in literal_arguments(file, literal) {
            if !entries.contains_key(&found.value) {
                out.push(Violation {
                    path: file.path.clone(),
                    line: found.at.0,
                    column: found.at.1,
                    rule_id: rule.id.to_string(),
                    message: format!("`{}` passed to `{}` is not listed in {}", found.value, found.callee, literal.file),
                });
            }
        }
    }
}

/// The string literals passed as the watched argument of the watched calls.
pub fn literal_arguments(file: &SourceFile, literal: &CompiledLiteral) -> Vec<Literal> {
    let adapter = adapters::for_language(file.language);
    let src = file.text.as_str();
    let mut found = Vec::new();
    visit(file.tree.root_node(), &mut |node| {
        let Some(callee) = adapter.call_name(node, src) else { return };
        if !literal.calls.is_match(&callee) {
            return;
        }
        let Some(argument) = adapter.call_arguments(node).and_then(|args| args.get(literal.position).copied()) else { return };
        let Some(value) = adapter.string_value(argument, src) else { return };
        let at = argument.start_position();
        found.push(Literal { value, at: (at.row + 1, at.column + 1), callee });
    });
    found
}

/// Registry entries that none of `files` passes to a watched call.
pub fn unused_registry_entries<'f>(
    rule: &CompiledRule,
    files: impl Iterator<Item = &'f SourceFile>,
    ctx: &Project,
) -> Vec<Violation> {
    let Some(literal) = rule.literal.as_ref().filter(|l| l.report_unused) else { return Vec::new() };
    let used: HashSet<String> = files.flat_map(|f| literal_arguments(f, literal)).map(|l| l.value).collect();
    literal
        .entries(ctx)
        .iter()
        .filter(|(name, _)| !used.contains(*name))
        .map(|(name, at)| Violation {
            path: literal.file.clone(),
            line: at.0,
            column: at.1,
            rule_id: rule.id.to_string(),
            message: format!("`{name}` is listed here but no watched call passes it"),
        })
        .collect()
}

/// Depth-first walk over `node` and all its descendants.
pub fn visit(node: Node, f: &mut impl FnMut(Node)) {
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
