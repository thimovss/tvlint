//! Checks of single statements and expressions: assignments, comparisons, call arguments, and what is raised.

use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::adapters::{Adapter, last_segment};
use super::{CompiledRule, structure};

/// Whether the rule has any check that `check` runs.
pub(super) fn applies(rule: &CompiledRule) -> bool {
    !rule.assign.is_empty()
        || !rule.augmented.is_empty()
        || rule.literal_compared
        || !rule.literal_assigned.is_empty()
        || rule.require_argument.is_some()
        || rule.forbid_argument.is_some()
        || rule.argument_keys.is_some()
        || rule.allow_raise.is_some()
}

/// Runs the statement and expression checks on `node`. `raise_classes` holds the classes `allow_raise` accepts.
pub(super) fn check(
    rule: &CompiledRule,
    adapter: &dyn Adapter,
    node: Node,
    src: &str,
    raise_classes: Option<&BTreeSet<String>>,
    report: &mut impl FnMut(Node, String),
) {
    if !applies(rule) || !structure::within_function(rule, adapter, node, src) {
        return;
    }
    assignment(rule, adapter, node, src, report);
    comparison(rule, adapter, node, src, report);
    arguments(rule, adapter, node, src, report);
    raising(rule, adapter, node, src, raise_classes, report);
}

fn assignment(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str, report: &mut impl FnMut(Node, String)) {
    if rule.assign.is_empty() && rule.augmented.is_empty() && rule.literal_assigned.is_empty() {
        return;
    }
    let Some(assignment) = adapter.assignment(node, src) else { return };
    let target = assignment.target.as_str();
    if !assignment.declaration && !assignment.augmented && rule.assign.is_match(target) {
        report(node, format!("assigning to `{target}` is forbidden"));
    } else if assignment.augmented && (rule.assign.is_match(target) || rule.augmented.is_match(target)) {
        report(node, format!("augmented assignment to `{target}` is forbidden"));
    }
    if !assignment.augmented
        && rule.literal_assigned.is_match(target)
        && let Some(value) = assignment.value
        && let Some(literal) = adapter.string_value(value, src)
    {
        report(value, format!("the string literal \"{literal}\" is assigned to `{target}`; use an enum or a Literal type"));
    }
}

fn comparison(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str, report: &mut impl FnMut(Node, String)) {
    if !rule.literal_compared {
        return;
    }
    let Some(comparison) = adapter.comparison(node, src) else { return };
    if !comparison.operators.iter().any(|o| matches!(o.as_str(), "==" | "!=" | "===" | "!==")) {
        return;
    }
    for operand in comparison.operands {
        if let Some(literal) = adapter.string_value(operand, src) {
            report(operand, format!("compared with the string literal \"{literal}\"; use an enum or a Literal type"));
        }
    }
}

fn arguments(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str, report: &mut impl FnMut(Node, String)) {
    if rule.require_argument.is_none() && rule.forbid_argument.is_none() && rule.argument_keys.is_none() {
        return;
    }
    let Some(callee) = adapter.call_name(node, src) else { return };
    let keywords = adapter.call_keywords(node, src);
    if let (Some(check), Some(keywords)) = (&rule.require_argument, &keywords)
        && check.calls.is_match(&callee)
        && !keywords.has_splat
    {
        for name in check.names.iter().filter(|n| !keywords.named.iter().any(|(k, _)| k == *n)) {
            report(node, format!("the call to `{callee}` does not pass `{name}`"));
        }
    }
    if let (Some(check), Some(keywords)) = (&rule.forbid_argument, &keywords)
        && check.calls.is_match(&callee)
    {
        for (name, value) in keywords.named.iter().filter(|(k, _)| check.names.contains(k)) {
            report(*value, format!("the call to `{callee}` must not pass `{name}`"));
        }
    }
    if let Some(keys) = &rule.argument_keys
        && keys.calls.is_match(&callee)
    {
        let argument = match &keys.keyword {
            Some(keyword) => keywords.as_ref().and_then(|k| k.named.iter().find(|(name, _)| name == keyword).map(|(_, value)| *value)),
            None => adapter.call_arguments(node).and_then(|args| args.get(keys.position).copied()),
        };
        for (key, at) in argument.and_then(|a| adapter.dict_entries(a, src)).unwrap_or_default() {
            if keys.forbid.is_match(&key) {
                report(at, format!("the key `{key}` is not allowed in the dict passed to `{callee}`"));
            } else if !keys.allow_text.is_empty() && !keys.allow.is_match(&key) {
                report(at, format!("the key `{key}` in the dict passed to `{callee}` must match {}", keys.allow_text.join(" or ")));
            }
        }
    }
}

/// `allow_raise`: anything raised that is neither listed nor a subclass of the allowed class. A name that does not start
/// with a capital is a variable (`raise error`), whose class is unknown, so it is left alone.
fn raising(
    rule: &CompiledRule,
    adapter: &dyn Adapter,
    node: Node,
    src: &str,
    raise_classes: Option<&BTreeSet<String>>,
    report: &mut impl FnMut(Node, String),
) {
    let Some(allow) = &rule.allow_raise else { return };
    let Some(name) = adapter.raised_name(node, src) else { return };
    let last = last_segment(&name);
    if !last.starts_with(|c: char| c.is_uppercase()) || allow.names.is_match(&name) || allow.names.is_match(last) {
        return;
    }
    if raise_classes.is_some_and(|classes| classes.contains(last)) {
        return;
    }
    let mut allowed = allow.names_text.clone();
    allowed.extend(allow.subclass_of.iter().map(|c| format!("{c} and its subclasses")));
    report(node, format!("raising `{name}` is not allowed here; allowed: {}", allowed.join(", ")));
}
