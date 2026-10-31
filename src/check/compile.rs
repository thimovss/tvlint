//! Rules with their patterns compiled, ready to run against files.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};

use crate::config::{ClassFilter, Lookup, Rule, Use};
use crate::data::{PathPattern, Position};
use super::Project;

/// A rule with its patterns compiled, ready to run against files.
pub struct CompiledRule<'a> {
    pub id: &'a str,
    pub rule: &'a Rule,
    /// The directory of the config that defines the rule, from the project root. Import patterns are relative to it.
    pub(super) base: String,
    pub(super) raise: GlobSet,
    pub(super) call: GlobSet,
    pub(super) catch_root: Option<&'a str>,
    pub(super) filename: GlobSet,
    pub(super) import: GlobSet,
    pub(super) except_import: GlobSet,
    pub(super) element: GlobSet,
    /// `forbid_import` or `except_import` has `{n}` placeholders, so they are compiled per file.
    pub(super) templated: bool,
    pub(super) require_call: GlobSet,
    pub(super) within_function: GlobSet,
    pub(super) except_function: GlobSet,
    pub(super) require_method: Vec<NamedGlob>,
    pub(super) require_field: Vec<NamedGlob>,
    pub(super) forbid_field: GlobSet,
    pub(super) class_filter: Option<CompiledClassFilter<'a>>,
    pub(super) assign: GlobSet,
    pub(super) augmented: GlobSet,
    pub(super) literal_compared: bool,
    pub(super) literal_assigned: GlobSet,
    pub(super) require_argument: Option<CompiledArguments>,
    pub(super) forbid_argument: Option<CompiledArguments>,
    pub(super) argument_keys: Option<CompiledKeys>,
    pub(super) allow_raise: Option<CompiledAllowRaise<'a>>,
    pub(super) property: HashSet<&'a str>,
    pub(super) within: HashSet<&'a str>,
    pub(super) property_value: Option<CompiledPropertyValue<'a>>,
    // Data files (JSON, YAML, TOML).
    pub(super) data_property: Vec<PathPattern>,
    pub(super) data_value: Option<CompiledDataValue>,
    pub(super) require_property: Vec<RequiredProperty>,
    pub(super) reference: Option<String>,
    pub literal: Option<CompiledLiteral>,
}

/// A name glob that remembers how it was written, for messages.
pub(super) struct NamedGlob {
    pub(super) text: String,
    pub(super) matcher: GlobMatcher,
}

pub(super) struct CompiledClassFilter<'a> {
    pub(super) subclass_of: Option<&'a str>,
    pub(super) names: GlobSet,
    pub(super) has_names: bool,
}

fn named_globs(patterns: &[String]) -> Result<Vec<NamedGlob>> {
    patterns
        .iter()
        .map(|text| Ok(NamedGlob { text: text.clone(), matcher: Glob::new(text)?.compile_matcher() }))
        .collect()
}

pub(super) struct CompiledDataValue {
    pub(super) names: Vec<PathPattern>,
    pub(super) forbid: GlobSet,
    pub(super) require: GlobSet,
}

pub(super) struct RequiredProperty {
    pub(super) parent: PathPattern,
    pub(super) key: String,
    pub(super) text: String,
}

pub(super) struct CompiledArguments {
    pub(super) calls: GlobSet,
    pub(super) names: Vec<String>,
}

pub(super) struct CompiledKeys {
    pub(super) calls: GlobSet,
    pub(super) position: usize,
    pub(super) keyword: Option<String>,
    pub(super) forbid: GlobSet,
    pub(super) allow: GlobSet,
    pub(super) allow_text: Vec<String>,
}

pub(super) struct CompiledAllowRaise<'a> {
    pub(super) names: GlobSet,
    pub(super) names_text: Vec<String>,
    pub(super) subclass_of: Option<&'a str>,
}

pub struct CompiledLiteral {
    /// The watched calls (`literal_argument`) or assignment targets (`literal_assignment`).
    pub(super) watched: GlobSet,
    pub(super) assignment: bool,
    pub(super) position: usize,
    /// The registry file, if the literals must be listed in one.
    pub file: Option<String>,
    registry_path: Option<PathPattern>,
    use_: Use,
    pub report_unused: bool,
    pub(super) pattern: GlobSet,
    pub(super) pattern_text: Vec<String>,
    pub(super) require_literal: bool,
    pub(super) lookup: Lookup,
    entries: OnceLock<BTreeMap<String, Position>>,
}

impl CompiledLiteral {
    /// The allowed values, by where they are defined in the registry file.
    pub fn entries(&self, ctx: &Project) -> &BTreeMap<String, Position> {
        self.entries.get_or_init(|| {
            let (Some(file), Some(path)) = (&self.file, &self.registry_path) else { return BTreeMap::new() };
            ctx.docs.get(file).map(|doc| doc.registry(path, self.use_)).unwrap_or_default()
        })
    }
}

/// A value a `literal_argument` or `literal_assignment` rule watches: `None` when it is not a plain string literal.
pub struct Literal {
    pub value: Option<String>,
    pub at: Position,
    /// The call, or the assignment target.
    pub watched: String,
}

fn compile_class_filter(filter: &ClassFilter) -> Result<CompiledClassFilter<'_>> {
    Ok(CompiledClassFilter { subclass_of: filter.subclass_of.as_deref(), names: compile(&filter.names)?, has_names: !filter.names.is_empty() })
}

fn compile_arguments(check: &crate::config::ArgumentCheck) -> Result<CompiledArguments> {
    Ok(CompiledArguments { calls: compile(&check.calls)?, names: check.names.clone() })
}

pub(super) struct CompiledPropertyValue<'a> {
    pub(super) names: HashSet<&'a str>,
    pub(super) in_call: GlobSet,
    pub(super) forbid_literal: bool,
    pub(super) starts_with: Option<GlobMatcher>,
}

/// `./a/b.json` and `a/b.json` name the same file.
pub fn normalize_file(path: &str) -> String {
    path.trim_start_matches("./").to_string()
}

pub(super) fn compile_value_globs(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

impl<'a> CompiledRule<'a> {
    pub fn new(id: &'a str, rule: &'a Rule) -> Result<Self> {
        let data = rule.language.iter().any(|l| l.is_data());
        let paths = |patterns: &[String], key: &str| -> Result<Vec<PathPattern>> {
            patterns
                .iter()
                .map(|p| PathPattern::parse(p))
                .collect::<Result<_>>()
                .with_context(|| format!("rule `{id}` {key}"))
        };
        Ok(CompiledRule {
            data_property: if data { paths(&rule.forbid_property, "forbid_property")? } else { Vec::new() },
            data_value: match (&rule.property_value, data) {
                (Some(pv), true) => Some(CompiledDataValue {
                    names: paths(&pv.names, "property_value names")?,
                    forbid: compile_value_globs(&pv.forbid_value).with_context(|| format!("rule `{id}` forbid_value"))?,
                    require: compile_value_globs(&pv.require_value).with_context(|| format!("rule `{id}` require_value"))?,
                }),
                _ => None,
            },
            require_property: rule
                .require_property
                .iter()
                .map(|text| {
                    let pattern = PathPattern::parse(text).with_context(|| format!("rule `{id}` require_property"))?;
                    let (parent, key) = pattern.split_last().with_context(|| {
                        format!("rule `{id}` require_property `{text}`: the last segment must be a key name, not a wildcard")
                    })?;
                    Ok(RequiredProperty { parent, key, text: text.clone() })
                })
                .collect::<Result<_>>()?,
            reference: rule.same_keys_as.as_deref().map(normalize_file),
            literal: rule
                .literal_check()
                .map(|arg| -> Result<CompiledLiteral> {
                    let assignment = rule.literal_assignment.is_some();
                    Ok(CompiledLiteral {
                        watched: compile(if assignment { &arg.names } else { &arg.calls })?,
                        assignment,
                        position: arg.position,
                        file: arg.registry.as_ref().map(|r| normalize_file(&r.file)),
                        registry_path: arg.registry.as_ref().map(|r| PathPattern::parse(&r.path)).transpose()?,
                        use_: arg.registry.as_ref().map_or(Use::Value, |r| r.use_),
                        report_unused: arg.report_unused,
                        pattern: compile(&arg.pattern)?,
                        pattern_text: arg.pattern.clone(),
                        require_literal: arg.require_literal,
                        lookup: arg.lookup,
                        entries: OnceLock::new(),
                    })
                })
                .transpose()
                .with_context(|| format!("rule `{id}` literal check"))?,
            assign: compile(&rule.forbid_assignment).with_context(|| format!("rule `{id}` forbid_assignment"))?,
            augmented: compile(&rule.forbid_augmented_assignment).with_context(|| format!("rule `{id}` forbid_augmented_assignment"))?,
            literal_compared: rule.forbid_literal.as_ref().is_some_and(|l| l.compared),
            literal_assigned: compile(rule.forbid_literal.as_ref().map_or(&[][..], |l| &l.assigned_to)).with_context(|| format!("rule `{id}` forbid_literal"))?,
            require_argument: rule.require_argument.as_ref().map(compile_arguments).transpose().with_context(|| format!("rule `{id}` require_argument"))?,
            forbid_argument: rule.forbid_argument.as_ref().map(compile_arguments).transpose().with_context(|| format!("rule `{id}` forbid_argument"))?,
            argument_keys: rule
                .argument_keys
                .as_ref()
                .map(|k| -> Result<CompiledKeys> {
                    Ok(CompiledKeys {
                        calls: compile(&k.calls)?,
                        position: k.position,
                        keyword: k.keyword.clone(),
                        forbid: compile(&k.forbid_keys)?,
                        allow: compile(&k.allow_keys)?,
                        allow_text: k.allow_keys.clone(),
                    })
                })
                .transpose()
                .with_context(|| format!("rule `{id}` argument_keys"))?,
            allow_raise: rule
                .allow_raise
                .as_ref()
                .map(|a| -> Result<CompiledAllowRaise> {
                    Ok(CompiledAllowRaise { names: compile(&a.names)?, names_text: a.names.clone(), subclass_of: a.subclass_of.as_deref() })
                })
                .transpose()
                .with_context(|| format!("rule `{id}` allow_raise"))?,
            id,
            rule,
            base: String::new(),
            raise: compile(&rule.forbid_raise).with_context(|| format!("rule `{id}` forbid_raise"))?,
            call: compile(&rule.forbid_call).with_context(|| format!("rule `{id}` forbid_call"))?,
            catch_root: rule.forbid_catch.as_ref().map(|c| c.subclass_of.as_str()),
            filename: compile(&rule.require_filename).with_context(|| format!("rule `{id}` require_filename"))?,
            import: compile_paths(&rule.forbid_import).with_context(|| format!("rule `{id}` forbid_import"))?,
            except_import: compile_paths(&rule.except_import).with_context(|| format!("rule `{id}` except_import"))?,
            element: compile(&rule.forbid_element).with_context(|| format!("rule `{id}` forbid_element"))?,
            templated: rule.max_placeholder() > 0,
            require_call: compile(&rule.require_call).with_context(|| format!("rule `{id}` require_call"))?,
            within_function: compile(&rule.within_function).with_context(|| format!("rule `{id}` within_function"))?,
            except_function: compile(&rule.except_function).with_context(|| format!("rule `{id}` except_function"))?,
            require_method: named_globs(&rule.require_method).with_context(|| format!("rule `{id}` require_method"))?,
            require_field: named_globs(&rule.require_field).with_context(|| format!("rule `{id}` require_field"))?,
            forbid_field: compile(&rule.forbid_field).with_context(|| format!("rule `{id}` forbid_field"))?,
            class_filter: rule
                .in_class
                .as_ref()
                .map(compile_class_filter)
                .transpose()
                .with_context(|| format!("rule `{id}` in_class"))?,
            property: rule.forbid_property.iter().map(String::as_str).collect(),
            within: rule.within_property.iter().map(String::as_str).collect(),
            property_value: rule
                .property_value
                .as_ref()
                .map(|pv| -> Result<CompiledPropertyValue> {
                    Ok(CompiledPropertyValue {
                        names: pv.names.iter().map(String::as_str).collect(),
                        in_call: compile(&pv.in_call)?,
                        forbid_literal: pv.forbid_literal,
                        starts_with: pv.array_starts_with.as_deref().map(|p| Glob::new(p).map(|g| g.compile_matcher())).transpose()?,
                    })
                })
                .transpose()
                .with_context(|| format!("rule `{id}` property_value"))?,
        })
    }
}

impl CompiledRule<'_> {
    /// Makes import patterns relative to `base`, the directory of the config the rule was written in.
    pub fn in_dir(mut self, base: &str) -> Self {
        self.base = base.to_string();
        self
    }
}

/// `path` relative to the directory `base` (both from the project root); `../` leads out of `base`.
pub(super) fn relative_to(base: &str, path: &str) -> String {
    if base.is_empty() {
        return path.to_string();
    }
    let base: Vec<&str> = base.split('/').collect();
    let parts: Vec<&str> = path.split('/').collect();
    let common = base.iter().zip(&parts).take_while(|(a, b)| a == b).count();
    let mut out = vec![".."; base.len() - common];
    out.extend(&parts[common..]);
    out.join("/")
}

pub(super) fn compile(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

/// Path-style globs: `*` stays within one segment and `**` crosses segments, as in scopes.
pub(super) fn compile_paths(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(GlobBuilder::new(&pattern.replace("::", "/")).literal_separator(true).build()?);
    }
    Ok(builder.build()?)
}
