//! Checks of the shape of code: which classes declare what, and which functions call what.

use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::adapters::Adapter;
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
    if let Some(filter) = &rule.class_filter {
        if let Some(root) = filter.subclass_of
            && (name == root || !subclasses.is_some_and(|s| s.contains(&name)))
        {
            return;
        }
        if filter.has_names && !filter.names.is_match(&name) {
            return;
        }
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

fn quoted(patterns: &[String]) -> String {
    patterns.iter().map(|p| format!("`{p}`")).collect::<Vec<_>>().join(", ")
}
