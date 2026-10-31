//! Checks of JSON, YAML and TOML files, run against the value model.

use std::collections::BTreeSet;

use crate::data::{self, Position};
use crate::source::SourceFile;
use super::{CompiledRule, Project, Violation};

pub(super) fn check_data(file: &SourceFile, rule: &CompiledRule, ctx: &Project, out: &mut Vec<Violation>) {
    let Some(doc) = ctx.docs.get(&file.path) else { return };
    let mut report = |at: Position, message: String| {
        out.push(Violation { path: file.path.clone(), line: at.0, column: at.1, rule_id: rule.id.to_string(), message });
    };

    let mut seen_parent = vec![false; rule.require_property.len()];
    doc.walk(&mut |path, at, node| {
        for (required, seen) in rule.require_property.iter().zip(seen_parent.iter_mut()) {
            if !required.parent.matches(path) {
                continue;
            }
            *seen = true;
            if let Some(entries) = node.map()
                && !entries.iter().any(|e| e.key == required.key)
            {
                let place = if path.is_empty() { "the file".to_string() } else { format!("`{}`", data::dotted(path)) };
                report(at, format!("{place} is missing the required `{}` property", required.key));
            }
        }
        if path.is_empty() {
            return;
        }
        if rule.data_property.iter().any(|p| p.matches(path)) {
            report(at, format!("the `{}` property is forbidden", data::dotted(path)));
        }
        let Some(values) = &rule.data_value else { return };
        if !values.names.iter().any(|p| p.matches(path)) {
            return;
        }
        let Some(scalar) = node.scalar() else { return };
        let (pv_forbid, pv_require) = (rule.rule.property_value.as_ref().map(|p| &p.forbid_value), rule.rule.property_value.as_ref().map(|p| &p.require_value));
        if values.forbid.is_match(&scalar.text) {
            let matching = values.forbid.matches(&scalar.text).first().and_then(|i| pv_forbid.and_then(|f| f.get(*i)));
            report(
                at,
                format!(
                    "`{}` is `{}`, which is forbidden{}",
                    data::dotted(path),
                    scalar.text,
                    matching.map_or(String::new(), |m| format!(" (matches `{m}`)"))
                ),
            );
        }
        if !values.require.is_empty() && !values.require.is_match(&scalar.text) {
            report(
                at,
                format!(
                    "`{}` is `{}`; it must match {}",
                    data::dotted(path),
                    scalar.text,
                    pv_require.map_or(String::new(), |r| r.iter().map(|g| format!("`{g}`")).collect::<Vec<_>>().join(" or "))
                ),
            );
        }
    });
    // A required key under a parent path that does not exist at all. Wildcard parents may match nothing.
    for (required, seen) in rule.require_property.iter().zip(seen_parent) {
        if !seen && required.parent.is_literal() {
            report((1, 1), format!("the required property `{}` is missing", required.text));
        }
    }

    if let Some(reference) = &rule.reference
        && *reference != file.path
        && let Some(expected) = ctx.docs.get(reference)
    {
        let want = expected.leaves();
        let have = doc.leaves();
        for key in want.keys().filter(|k| !have.contains_key(*k)) {
            report(doc.deepest_existing(key), format!("`{}` is missing; {reference} has it", data::dotted(key)));
        }
        for (key, (at, _)) in have.iter().filter(|(k, _)| !want.contains_key(*k)) {
            report(*at, format!("`{}` is not in {reference}", data::dotted(key)));
        }
        if rule.rule.same_placeholders {
            for (key, (at, text)) in &have {
                let (Some(text), Some((_, Some(expected_text)))) = (text, want.get(key)) else { continue };
                let (found, wanted) = (data::placeholders(text), data::placeholders(expected_text));
                if found != wanted {
                    let show = |set: &BTreeSet<String>| {
                        if set.is_empty() { "none".to_string() } else { set.iter().map(|p| format!("{{{p}}}")).collect::<Vec<_>>().join(", ") }
                    };
                    report(*at, format!("`{}` has placeholders {}, but {reference} has {}", data::dotted(key), show(&found), show(&wanted)));
                }
            }
        }
    }
}
