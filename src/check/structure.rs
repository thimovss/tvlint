//! Checks of the shape of code: which classes declare what, and which functions call what.

use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::adapters::Adapter;
use super::compile::{CompiledClassFilter, CompiledFunctions, CompiledRequireExport};
use super::{CompiledRule, Project, visit};

/// Whether `node` sits inside a function the rule's `within_function` selects.
pub(super) fn within_function(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str) -> bool {
    if rule.within_function.is_empty() {
        return true;
    }
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if adapter.function_name(ancestor, src).is_some_and(|name| selected_function(rule, &name)) {
            return true;
        }
        current = ancestor.parent();
    }
    false
}

fn selected_function(rule: &CompiledRule, name: &str) -> bool {
    rule.within_function.is_match(name) && !rule.except_function.is_match(name)
}

/// `require_call` applied to one function: reports it when its body holds none of the required calls.
pub(super) fn check_function(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str, report: &mut impl FnMut(Node, String)) {
    if rule.require_call.is_empty() {
        return;
    }
    let Some(name) = adapter.function_name(node, src).filter(|name| selected_function(rule, name)) else { return };
    let mut called = false;
    visit(node, &mut |inner| {
        called |= adapter.call_name(inner, src).is_some_and(|callee| rule.require_call.is_match(&callee));
    });
    if !called {
        report(node, format!("function `{name}` never calls {}", quoted(&rule.rule.require_call)));
    }
}

/// The classes `in_class` selects, other than the one that is named as the root.
pub(super) fn class_scope(rule: &CompiledRule, adapter: &dyn Adapter, ctx: &Project) -> Option<BTreeSet<String>> {
    let root = rule.class_filter.as_ref()?.subclass_of?;
    Some(ctx.index.subclasses_of(adapter.class_family()?, root))
}

/// Whether the class called `name` is one the filter selects. `subclasses` is what `subclasses` returned.
pub(super) fn selects(filter: &CompiledClassFilter, name: &str, subclasses: Option<&BTreeSet<String>>) -> bool {
    if let Some(root) = filter.subclass_of
        && (name == root || !subclasses.is_some_and(|s| s.contains(name)))
    {
        return false;
    }
    !filter.has_names || filter.names.is_match(name)
}

/// `require_method`, `require_field` and `forbid_field` applied to one class.
pub(super) fn check_class(
    rule: &CompiledRule,
    adapter: &dyn Adapter,
    node: Node,
    src: &str,
    subclasses: Option<&BTreeSet<String>>,
    report: &mut impl FnMut(Node, String),
) {
    if rule.require_method.is_empty() && rule.require_field.is_empty() && rule.forbid_field.is_empty() {
        return;
    }
    let (Some((name, _)), Some(members)) = (adapter.class_bases(node, src), adapter.class_members(node, src)) else { return };
    if let Some(filter) = &rule.class_filter
        && !selects(filter, &name, subclasses)
    {
        return;
    }
    for wanted in &rule.require_method {
        if !members.methods.iter().any(|m| wanted.matcher.is_match(m)) {
            report(node, format!("class `{name}` does not declare the method `{}`", wanted.text));
        }
    }
    for wanted in &rule.require_field {
        if !members.fields.iter().any(|f| wanted.matcher.is_match(&f.name)) {
            report(node, format!("class `{name}` does not declare the field `{}`", wanted.text));
        }
    }
    for field in members.fields.iter().filter(|f| rule.forbid_field.is_match(&f.name)) {
        report(field.node, format!("class `{name}` must not declare the field `{}`", field.name));
    }
}

/// The name of the function `node` defines, if `max_functions` counts it: it is at the top level of the file
/// (not inside another function or class), its name matches and it contains one of the elements asked for.
pub(super) fn counted_function(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str) -> Option<String> {
    let counted = rule.max_functions.as_ref()?;
    let name = adapter.function_name(node, src).filter(|name| counted.names.is_match(name))?;
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if adapter.function_name(parent, src).is_some() || adapter.class_bases(parent, src).is_some() {
            return None;
        }
        ancestor = parent.parent();
    }
    if !counted.spec.containing_element.is_empty() {
        let mut found = false;
        visit(node, &mut |inner| {
            found |= adapter.element_name(inner, src).is_some_and(|element| counted.elements.is_match(&element));
        });
        if !found {
            return None;
        }
    }
    Some(name)
}

/// Reports every function that `max_functions` counts beyond the allowed number.
pub(super) fn report_excess(counted: &CompiledFunctions, found: &[(Node, String)], report: &mut impl FnMut(Node, String)) {
    let spec = counted.spec;
    if found.len() <= spec.max {
        return;
    }
    let mut kind = String::new();
    if !spec.names.is_empty() {
        kind.push_str(&format!(" named {}", spec.names.join("/")));
    }
    if !spec.containing_element.is_empty() {
        kind.push_str(&format!(" containing the element {}", spec.containing_element.join("/")));
    }
    for (i, (node, name)) in found.iter().enumerate().skip(spec.max) {
        report(*node, format!("`{name}` is function {} of {} at the top level of the file{kind}; at most {} allowed", i + 1, found.len(), spec.max));
    }
}

/// `forbid_export` applied to the exports of one statement; `met` is set when `require_export` is satisfied.
pub(super) fn check_exports(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str, met: &mut bool, report: &mut impl FnMut(Node, String)) {
    if rule.forbid_export.is_none() && rule.require_export.is_none() {
        return;
    }
    for export in adapter.exports(node, src) {
        let shown = if export.name.is_empty() { "an anonymous value".to_string() } else { format!("`{}`", export.name) };
        if let Some(forbid) = &rule.forbid_export
            && forbid.names.is_match(&export.name)
            && (!forbid.spec.declared || export.declared)
            && forbid.spec.default.is_none_or(|default| default == export.default)
        {
            let kind = if export.default { "default" } else { "named" };
            let how = if export.declared { " where it is declared" } else { "" };
            report(export.node, format!("exporting {shown} as a {kind} export{how} is forbidden"));
        }
        if let Some(require) = &rule.require_export {
            *met |= (!require.spec.default || export.default) && require.names.is_match(&export.name);
        }
    }
}

/// What a file must export, for the message when it does not.
pub(super) fn missing_export(require: &CompiledRequireExport) -> String {
    let kind = if require.spec.default { "default export" } else { "export" };
    if require.spec.names.is_empty() {
        format!("the file has no {kind}")
    } else {
        format!("the file has no {kind} named {}", require.spec.names.join(" or "))
    }
}

fn quoted(patterns: &[String]) -> String {
    patterns.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ")
}
