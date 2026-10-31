use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use rayon::prelude::*;
use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use tree_sitter::Node;

use crate::adapters::{self, Adapter, last_segment};
use crate::config::{Rule, Use};
use crate::data::{self, Doc, PathPattern, Position};
use crate::source::SourceFile;

/// What a check needs to know about the rest of the project.
#[derive(Clone, Copy)]
pub struct Project<'a> {
    pub index: &'a ClassIndex,
    /// Import prefix -> project path, e.g. `@/` -> `src/`.
    pub aliases: &'a BTreeMap<String, String>,
    /// What `tsconfig.json`, `go.mod` and `pyproject.toml` say about where imports lead. The `aliases` of the rule's config win over tsconfig.
    pub modules: &'a crate::modules::Modules,
    /// Every file in the project, to find which file an import points at.
    pub paths: &'a HashSet<String>,
    /// Every JSON, YAML and TOML file, parsed.
    pub docs: &'a BTreeMap<String, Doc>,
}

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
    /// The directory of the config that defines the rule, from the project root. Import patterns are relative to it.
    base: String,
    raise: GlobSet,
    call: GlobSet,
    catch_root: Option<&'a str>,
    filename: GlobSet,
    import: GlobSet,
    except_import: GlobSet,
    element: GlobSet,
    property: HashSet<&'a str>,
    within: HashSet<&'a str>,
    property_value: Option<CompiledPropertyValue<'a>>,
    // Data files (JSON, YAML, TOML).
    data_property: Vec<PathPattern>,
    data_value: Option<CompiledDataValue>,
    require_property: Vec<RequiredProperty>,
    reference: Option<String>,
    pub literal: Option<CompiledLiteral>,
}

struct CompiledDataValue {
    names: Vec<PathPattern>,
    forbid: GlobSet,
    require: GlobSet,
}

struct RequiredProperty {
    parent: PathPattern,
    key: String,
    text: String,
}

pub struct CompiledLiteral {
    calls: GlobSet,
    position: usize,
    pub file: String,
    registry_path: PathPattern,
    use_: Use,
    pub report_unused: bool,
    entries: OnceLock<BTreeMap<String, Position>>,
}

impl CompiledLiteral {
    /// The allowed values, by where they are defined in the registry file.
    pub fn entries(&self, ctx: &Project) -> &BTreeMap<String, Position> {
        self.entries.get_or_init(|| ctx.docs.get(&self.file).map(|doc| doc.registry(&self.registry_path, self.use_)).unwrap_or_default())
    }
}

/// A string literal passed as an argument that a `literal_argument` rule watches.
pub struct Literal {
    pub value: String,
    pub at: Position,
    pub callee: String,
}

struct CompiledPropertyValue<'a> {
    names: HashSet<&'a str>,
    in_call: GlobSet,
    forbid_literal: bool,
    starts_with: Option<GlobMatcher>,
}

/// `./a/b.json` and `a/b.json` name the same file.
pub fn normalize_file(path: &str) -> String {
    path.trim_start_matches("./").to_string()
}

fn compile_value_globs(patterns: &[String]) -> Result<GlobSet> {
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
                .literal_argument
                .as_ref()
                .map(|arg| -> Result<CompiledLiteral> {
                    Ok(CompiledLiteral {
                        calls: compile(&arg.calls)?,
                        position: arg.position,
                        file: normalize_file(&arg.registry.file),
                        registry_path: PathPattern::parse(&arg.registry.path)?,
                        use_: arg.registry.use_,
                        report_unused: arg.report_unused,
                        entries: OnceLock::new(),
                    })
                })
                .transpose()
                .with_context(|| format!("rule `{id}` literal_argument"))?,
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
fn relative_to(base: &str, path: &str) -> String {
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

fn compile(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

/// Path-style globs: `*` stays within one segment and `**` crosses segments, as in scopes.
fn compile_paths(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(GlobBuilder::new(&pattern.replace("::", "/")).literal_separator(true).build()?);
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
        let per_file: Vec<Vec<(String, Vec<String>)>> = files
            .par_iter()
            .map(|file| {
                let adapter = adapters::for_language(file.language);
                let mut classes = Vec::new();
                if adapter.indexes_classes() {
                    visit(file.tree.root_node(), &mut |node| {
                        if let Some(class) = adapter.class_bases(node, &file.text) {
                            classes.push(class);
                        }
                    });
                }
                classes
            })
            .collect();
        let mut index = ClassIndex::default();
        for (name, bases) in per_file.into_iter().flatten() {
            index.bases.entry(name).or_default().extend(bases);
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

/// Whether `node` sits inside an object-literal property named by the rule's `within_property`.
fn within_property(rule: &CompiledRule, adapter: &dyn Adapter, node: Node, src: &str) -> bool {
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

pub fn check_file(
    file: &SourceFile,
    rule: &CompiledRule,
    ctx: &Project,
    out: &mut Vec<Violation>,
) {
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
    let catch_classes = rule.catch_root.map(|root| (root, ctx.index.subclasses_of(root)));
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
            && within_property(rule, adapter, node, src)
        {
            report(node, format!("raising `{name}` is forbidden"));
        }
        if !rule.call.is_empty()
            && let Some(name) = adapter.call_name(node, src)
            && rule.call.is_match(&name)
            && within_property(rule, adapter, node, src)
        {
            report(node, format!("calling `{name}` is forbidden"));
        }
        if !rule.import.is_empty() {
            // Rust writes paths with `::`; matching treats them as `/` like every other path.
            let key = |s: &str| s.replace("::", "/");
            for import in adapter.imports(node, src, &file.path, ctx) {
                // A package name stays as written; a project path is relative to the rule's config.
                let resolved = if import.resolved == import.specifier { key(&import.resolved) } else { relative_to(&rule.base, &key(&import.resolved)) };
                let hits = |set: &GlobSet| set.is_match(key(&import.specifier)) || set.is_match(&resolved);
                if hits(&rule.import) && !hits(&rule.except_import) {
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

fn check_data(file: &SourceFile, rule: &CompiledRule, ctx: &Project, out: &mut Vec<Violation>) {
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

#[cfg(test)]
mod tests;
