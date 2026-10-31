use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use rayon::prelude::*;
use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use tree_sitter::Node;

use crate::adapter::{self, Adapter, last_segment};
use crate::config::{Rule, Use};
use crate::data::{self, Doc, PathPattern, Position};
use crate::source::SourceFile;

/// What a check needs to know about the rest of the project.
#[derive(Clone, Copy)]
pub struct Project<'a> {
    pub index: &'a ClassIndex,
    /// Import prefix -> project path, e.g. `@/` -> `src/`.
    pub aliases: &'a BTreeMap<String, String>,
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
                let adapter = adapter::for_language(file.language);
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
    if file.language.is_data() {
        check_data(file, rule, ctx, out);
        return;
    }
    let adapter = adapter::for_language(file.language);
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
    let adapter = adapter::for_language(file.language);
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
mod tests {
    use super::*;
    use crate::config::{ClassSelector, Language};
    use crate::data::Doc;
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
            ..empty_rule()
        }
    }

    fn empty_rule() -> Rule {
        Rule {
            language: Vec::new(),
            forbid_raise: Vec::new(),
            forbid_call: Vec::new(),
            forbid_catch: None,
            forbid_import: Vec::new(),
            except_import: Vec::new(),
            forbid_element: Vec::new(),
            forbid_property: Vec::new(),
            property_value: None,
            within_property: Vec::new(),
            require_property: Vec::new(),
            same_keys_as: None,
            same_placeholders: false,
            literal_argument: None,
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
        hits_with(path, source, rule, &BTreeMap::new(), &HashSet::new())
    }

    fn hits_with(
        path: &str,
        source: &str,
        rule: &Rule,
        aliases: &BTreeMap<String, String>,
        paths: &HashSet<String>,
    ) -> Vec<(usize, String)> {
        let file = parse(path, source);
        let index = ClassIndex::build(std::slice::from_ref(&file));
        let compiled = CompiledRule::new("r", rule).unwrap();
        let docs = BTreeMap::new();
        let ctx = Project { index: &index, aliases, paths, docs: &docs };
        let mut out = Vec::new();
        check_file(&file, &compiled, &ctx, &mut out);
        out.into_iter().map(|v| (v.line, v.message)).collect()
    }

    /// Checks `target` among `files` (path, text), all visible to the rule as a project.
    fn project_hits(files: &[(&str, &str)], target: &str, rule: &Rule) -> Vec<(usize, usize, String)> {
        let parsed: Vec<SourceFile> = files.iter().map(|(p, t)| parse(p, t)).collect();
        let index = ClassIndex::build(&parsed);
        let docs: BTreeMap<String, Doc> = parsed.iter().filter(|f| f.language.is_data()).map(|f| (f.path.clone(), Doc::build(f))).collect();
        let compiled = CompiledRule::new("r", rule).unwrap();
        let (aliases, paths) = (BTreeMap::new(), HashSet::new());
        let ctx = Project { index: &index, aliases: &aliases, paths: &paths, docs: &docs };
        let mut out = Vec::new();
        check_file(parsed.iter().find(|f| f.path == target).unwrap(), &compiled, &ctx, &mut out);
        out.into_iter().map(|v| (v.line, v.column, v.message)).collect()
    }

    fn data_rule(language: Language) -> Rule {
        Rule { language: vec![language], ..empty_rule() }
    }

    fn property_value(names: &[&str], forbid: &[&str], require: &[&str]) -> Option<crate::config::PropertyValue> {
        let list = |items: &[&str]| items.iter().map(|s| s.to_string()).collect();
        Some(crate::config::PropertyValue {
            names: list(names),
            in_call: Vec::new(),
            forbid_literal: false,
            array_starts_with: None,
            forbid_value: list(forbid),
            require_value: list(require),
        })
    }

    // ---- JSON, YAML, TOML

    #[test]
    fn forbid_property_reports_the_key_at_its_path() {
        let json = "{\n  \"scripts\": {\n    \"postinstall\": \"x\",\n    \"build\": \"y\"\n  },\n  \"postinstall\": 1\n}\n";
        let r = Rule { forbid_property: vec!["scripts.postinstall".into()], ..data_rule(Language::Json) };
        let found = project_hits(&[("package.json", json)], "package.json", &r);
        assert_eq!(found, [(3, 5, "the `scripts.postinstall` property is forbidden".to_string())]);
        let yaml = "jobs:\n  a:\n    permissions: write-all\n  b:\n    runs-on: x\n";
        let r = Rule { forbid_property: vec!["jobs.*.permissions".into()], ..data_rule(Language::Yaml) };
        assert_eq!(project_hits(&[("w.yml", yaml)], "w.yml", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
        let toml = "[dependencies]\nserde = \"1\"\n[dev-dependencies]\nserde = \"1\"\n";
        let r = Rule { forbid_property: vec!["dev-dependencies".into()], ..data_rule(Language::Toml) };
        assert_eq!(project_hits(&[("Cargo.toml", toml)], "Cargo.toml", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn forbid_value_and_require_value_read_scalars_by_path() {
        let json = "{\"dependencies\": {\"a\": \"^1.0.0\", \"b\": \"1.2.3\", \"c\": \"latest\"}, \"strict\": false}";
        let r = Rule { property_value: property_value(&["dependencies.*"], &["^*", "~*", "latest"], &[]), ..data_rule(Language::Json) };
        let found = project_hits(&[("p.json", json)], "p.json", &r);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].2.contains("`dependencies.a` is `^1.0.0`") && found[0].2.contains("`^*`"), "{found:?}");
        let r = Rule { property_value: property_value(&["strict"], &[], &["true"]), ..data_rule(Language::Json) };
        let found = project_hits(&[("p.json", json)], "p.json", &r);
        assert_eq!(found.len(), 1);
        assert!(found[0].2.contains("`strict` is `false`; it must match `true`"), "{found:?}");
    }

    #[test]
    fn yaml_values_ignore_quotes_and_walk_lists() {
        let yaml = "services:\n  web:\n    image: \"app:latest\"\n  db:\n    image: pg:16\nsteps:\n  - uses: a/b@main\n  - uses: c/d@0123abc\n";
        let r = Rule { property_value: property_value(&["services.*.image"], &["*:latest"], &[]), ..data_rule(Language::Yaml) };
        assert_eq!(project_hits(&[("c.yaml", yaml)], "c.yaml", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
        let r = Rule { property_value: property_value(&["steps.*.uses"], &[], &["*@[0-9a-f][0-9a-f][0-9a-f]*"]), ..data_rule(Language::Yaml) };
        assert_eq!(project_hits(&[("c.yaml", yaml)], "c.yaml", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [7]);
    }

    #[test]
    fn require_property_checks_every_match_of_the_parent() {
        let yaml = "jobs:\n  build:\n    timeout-minutes: 5\n  test:\n    runs-on: x\n";
        let r = Rule { require_property: vec!["jobs.*.timeout-minutes".into()], ..data_rule(Language::Yaml) };
        let found = project_hits(&[("w.yml", yaml)], "w.yml", &r);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].0, found[0].2.as_str()), (4, "`jobs.test` is missing the required `timeout-minutes` property"));
        // A literal parent that is absent is reported once; a wildcard parent with no match is fine.
        let r = Rule { require_property: vec!["permissions.contents".into(), "services.*.image".into()], ..data_rule(Language::Yaml) };
        let found = project_hits(&[("w.yml", yaml)], "w.yml", &r);
        assert_eq!(found.len(), 1);
        assert!(found[0].2.contains("`permissions.contents` is missing"), "{found:?}");
        // The root counts as a parent, and every YAML document is checked.
        let r = Rule { require_property: vec!["name".into()], ..data_rule(Language::Yaml) };
        let found = project_hits(&[("k.yaml", "name: a\n---\nkind: b\n")], "k.yaml", &r);
        assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
        let r = Rule { require_property: vec!["package.license".into()], ..data_rule(Language::Toml) };
        assert!(project_hits(&[("Cargo.toml", "[package]\nname = \"x\"\nlicense = \"MIT\"\n")], "Cargo.toml", &r).is_empty());
    }

    fn locales(rule_placeholders: bool) -> Vec<(usize, usize, String)> {
        let en = "{\"title\": \"Hi {{name}}\", \"menu\": {\"home\": \"Home\", \"about\": \"About\"}}";
        let nl = "{\n  \"title\": \"Hallo\",\n  \"menu\": {\n    \"home\": \"Thuis\",\n    \"extra\": \"x\"\n  }\n}\n";
        let r = Rule { same_keys_as: Some("en.json".into()), same_placeholders: rule_placeholders, ..data_rule(Language::Json) };
        project_hits(&[("en.json", en), ("nl.json", nl)], "nl.json", &r)
    }

    #[test]
    fn same_keys_as_reports_missing_and_extra_keys() {
        let found = locales(false);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().any(|h| h.2 == "`menu.about` is missing; en.json has it" && h.0 == 3));
        assert!(found.iter().any(|h| h.2 == "`menu.extra` is not in en.json" && h.0 == 5));
    }

    #[test]
    fn same_placeholders_compares_shared_messages() {
        let found = locales(true);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.iter().any(|h| h.0 == 2 && h.2.contains("has placeholders none, but en.json has {name}")));
    }

    #[test]
    fn the_reference_file_is_not_compared_with_itself() {
        let r = Rule { same_keys_as: Some("./en.json".into()), ..data_rule(Language::Json) };
        assert!(project_hits(&[("en.json", "{\"a\": 1}")], "en.json", &r).is_empty());
    }

    // ---- Literal arguments against a registry

    fn literal_rule_for(language: Language, calls: &[&str], position: usize, use_: Use, path: &str) -> Rule {
        Rule {
            language: vec![language],
            literal_argument: Some(crate::config::LiteralArgument {
                calls: calls.iter().map(|s| s.to_string()).collect(),
                position,
                registry: crate::config::Registry { file: "codes.json".into(), path: path.into(), use_ },
                report_unused: true,
            }),
            ..empty_rule()
        }
    }

    #[test]
    fn literal_arguments_must_be_in_the_registry() {
        let codes = ("codes.json", "{\"codes\": [\"a_failed.x\", \"b_failed.y\"]}");
        let py = "raise Err('a_failed.x', {})\nraise Err('zzz', {})\nraise Err(code)\nraise Err(f'a{x}')\nraise Err(message='q')\nraise Other('nope')\n";
        let r = literal_rule_for(Language::Python, &["Err"], 0, Use::Value, "codes.*");
        let found = project_hits(&[codes, ("a.py", py)], "a.py", &r);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].0, found[0].2.as_str()), (2, "`zzz` passed to `Err` is not listed in codes.json"));
        let ts = "new Err('a_failed.x');\nthrow new Err(`b_failed.y`);\nnew Err('nope');\nnew Err(...rest, 'nope');\n";
        let r = literal_rule_for(Language::TypeScript, &["Err"], 0, Use::Value, "codes.*");
        assert_eq!(project_hits(&[codes, ("a.ts", ts)], "a.ts", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [3]);
        let rs = "fn f() { err(\"a_failed.x\", 1); err(\"bad\", 2); err(name, 3); }\n";
        let r = literal_rule_for(Language::Rust, &["err"], 0, Use::Value, "codes.*");
        assert_eq!(project_hits(&[codes, ("a.rs", rs)], "a.rs", &r).len(), 1);
    }

    #[test]
    fn a_later_argument_and_registry_keys() {
        let codes = ("codes.json", "{\"codes\": {\"home\": \"Home\", \"nav\": {\"about\": \"About\"}}}");
        let ts = "t('x', 'home');\nt('x', 'nope');\n";
        let r = literal_rule_for(Language::TypeScript, &["t"], 1, Use::Key, "codes.*");
        assert_eq!(project_hits(&[codes, ("a.ts", ts)], "a.ts", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [2]);
        let ts = "t('nav.about');\nt('nav');\nt('home');\n";
        let r = literal_rule_for(Language::TypeScript, &["t"], 0, Use::Path, "codes.**");
        assert_eq!(project_hits(&[codes, ("a.ts", ts)], "a.ts", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2, 3]);
        let r = literal_rule_for(Language::TypeScript, &["t"], 0, Use::Path, "**");
        assert_eq!(project_hits(&[codes, ("a.ts", ts)], "a.ts", &r).iter().map(|h| h.0).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn unused_registry_entries_are_found_across_files() {
        let json = "{\n  \"codes\": [\n    \"a\",\n    \"b\",\n    \"c\"\n  ]\n}\n";
        let files = [parse("codes.json", json), parse("x.py", "Err('a')\n"), parse("y.py", "Err('c')\nErr('c')\n")];
        let docs: BTreeMap<String, Doc> = [("codes.json".to_string(), Doc::build(&files[0]))].into();
        let r = literal_rule_for(Language::Python, &["Err"], 0, Use::Value, "codes.*");
        let compiled = CompiledRule::new("r", &r).unwrap();
        let index = ClassIndex::default();
        let (aliases, paths) = (BTreeMap::new(), HashSet::new());
        let ctx = Project { index: &index, aliases: &aliases, paths: &paths, docs: &docs };
        let unused = unused_registry_entries(&compiled, files[1..].iter(), &ctx);
        assert_eq!(unused.len(), 1);
        assert_eq!((unused[0].path.as_str(), unused[0].line, unused[0].message.as_str()), ("codes.json", 4, "`b` is listed here but no watched call passes it"));
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
            ("a.rs", Language::Rust),
        ] {
            assert_eq!(parse(path, "").language, language, "{path}");
        }
    }

    // ---- imports

    fn ts_rule() -> Rule {
        Rule { language: vec![Language::TypeScript], ..empty_rule() }
    }

    fn aliases() -> BTreeMap<String, String> {
        BTreeMap::from([("@/".to_string(), "src/".to_string())])
    }

    fn project(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn every_import_form_is_found() {
        let src = "import a from 'pkg';\nimport 'other';\nimport type { T } from 'pkg';\nexport * from 'pkg';\n\
export { x } from 'pkg';\nconst m = await import('pkg');\nconst r = require('pkg');\nimport e = require('pkg');\n";
        let r = Rule { forbid_import: vec!["pkg".into()], ..ts_rule() };
        assert_eq!(lines("t.ts", src, &r), [1, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn relative_to_steps_out_of_the_base_directory() {
        assert_eq!(relative_to("", "src/a.ts"), "src/a.ts");
        assert_eq!(relative_to("web", "web/src/a.ts"), "src/a.ts");
        assert_eq!(relative_to("web/app", "web/lib/a.ts"), "../lib/a.ts");
        assert_eq!(relative_to("web", "api/a.ts"), "../api/a.ts");
    }

    #[test]
    fn imports_resolve_aliases_relative_paths_and_extensions() {
        let paths = project(&[
            "src/components/ui/button.tsx",
            "src/hooks/use-mobile.ts",
            "src/parties/hooks/useParty.tsx",
            "src/utils/index.ts",
        ]);
        let src = "import { B } from '@/components/ui/button';\nimport u from '@/hooks/use-mobile';\n\
import p from '../../parties/hooks/useParty';\nimport x from '@/utils';\nimport R from 'react';\n";
        let r = Rule {
            forbid_import: vec!["src/**".into()],
            except_import: vec!["src/components/ui/**".into(), "src/utils/**".into()],
            ..ts_rule()
        };
        let found = hits_with("src/components/ui/sidebar.tsx", src, &r, &aliases(), &paths);
        assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 3]);
        assert!(found[0].1.contains("src/hooks/use-mobile.ts"), "{found:?}");
        assert!(found[1].1.contains("src/parties/hooks/useParty.tsx"), "{found:?}");
    }

    #[test]
    fn package_patterns_match_the_specifier_as_written() {
        let src = "import a from '@base-ui/react';\nimport b from '@base-ui/react/dialog';\nimport c from '@base-ui/reactish';\n";
        let r = Rule { forbid_import: vec!["@base-ui/react".into(), "@base-ui/react/**".into()], ..ts_rule() };
        assert_eq!(lines("t.ts", src, &r), [1, 2]);
    }

    #[test]
    fn imports_that_leave_the_project_are_left_alone() {
        let src = "import a from '../../../outside';\n";
        let r = Rule { forbid_import: vec!["**".into()], ..ts_rule() };
        // Still matched as the raw specifier, but never resolved to a bogus project path.
        assert_eq!(hits_with("src/a.ts", src, &r, &aliases(), &HashSet::new())[0].1, "importing `../../../outside` is forbidden");
    }

    // ---- JSX elements

    #[test]
    fn jsx_elements_are_matched_by_name() {
        let src = "const a = <button onClick={f}>x</button>;\nconst b = <input />;\nconst c = <Button />;\nconst d = <table.Row />;\n";
        let r = Rule { forbid_element: vec!["button".into(), "input".into(), "table".into()], ..ts_rule() };
        assert_eq!(lines("t.tsx", src, &r), [1, 2]);
    }

    // ---- object properties

    #[test]
    fn forbid_property_sees_object_keys_but_not_jsx_attributes() {
        let src = "useThing({ onError: () => {}, onSuccess });\nconst o = { onError() {}, 'onError': 1, onError };\n\
const p = <img onError={x} />;\nconst { onError: renamed } = o;\n";
        let r = Rule { forbid_property: vec!["onError".into()], ..ts_rule() };
        assert_eq!(lines("t.tsx", src, &r), [1, 2, 2, 2]);
    }

    #[test]
    fn within_property_only_counts_hits_inside_that_property() {
        let src = "mutate(v, {\n  onError: () => toast.add({ a: 1 }),\n  onSuccess: () => toast.add({ b: 1 }),\n});\ntoast.add({ c: 1 });\n";
        let r = Rule { forbid_call: vec!["toast.add".into()], within_property: vec!["onError".into()], ..ts_rule() };
        assert_eq!(lines("t.ts", src, &r), [2]);
    }

    fn literal_rule() -> Rule {
        Rule {
            property_value: Some(crate::config::PropertyValue {
                names: vec!["title".into(), "description".into()],
                in_call: vec!["toast.add".into()],
                forbid_literal: true,
                array_starts_with: None,
                forbid_value: Vec::new(),
                require_value: Vec::new(),
            }),
            ..ts_rule()
        }
    }

    #[test]
    fn forbid_literal_flags_text_that_skips_translation() {
        let src = "toast.add({ title: 'Hi', description: `plain`, type: 'error' });\n\
toast.add({ title: t('a'), description: `${n} files` });\nother({ title: 'Hi' });\nconst x = { title: 'Hi' };\n";
        assert_eq!(lines("t.tsx", src, &literal_rule()), [1, 1]);
    }

    #[test]
    fn array_starts_with_checks_literal_arrays_only() {
        let src = "useQuery({ queryKey: [accountID, 'a'] });\nuseQuery({ queryKey: ['x', accountID] });\n\
useQuery({ queryKey: key(accountID) });\nuseQuery({ queryKey: [] });\nuseQuery({ queryKey: [accountID] as const });\n\
useQuery({ queryKey: [...base, accountID] });\n";
        let r = Rule {
            property_value: Some(crate::config::PropertyValue {
                names: vec!["queryKey".into()],
                in_call: Vec::new(),
                forbid_literal: false,
                array_starts_with: Some("accountID".into()),
                forbid_value: Vec::new(),
                require_value: Vec::new(),
            }),
            ..ts_rule()
        };
        let found = hits("t.ts", src, &r);
        assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 4, 6]);
        assert!(found[1].1.contains("an empty array"), "{found:?}");
    }

    // ---- Rust

    fn rs_rule() -> Rule {
        Rule { language: vec![Language::Rust], ..empty_rule() }
    }

    #[test]
    fn rust_calls_cover_functions_methods_and_macros() {
        let src = "fn f() {\n    let a = Vec::new();\n    let b = x.unwrap();\n    let c = s.parse::<i32>().unwrap();\n\
    let d = foo()?.bar();\n    println!(\"hi\");\n    std::fs::read_to_string(\"p\").expect(\"e\");\n    panic!(\"no\");\n    (a.b)();\n    Vec::<i32>::new();\n}\n";
        let r = Rule {
            forbid_call: vec!["Vec::new".into(), "*.unwrap".into(), "*.bar".into(), "println!".into(), "std::fs::*".into(), "*.expect".into(), "panic!".into(), "a.b".into()],
            ..rs_rule()
        };
        // Vec::<i32>::new() is Vec::new too (generics are looked through).
        assert_eq!(lines("a.rs", src, &r), [2, 3, 4, 5, 6, 7, 7, 8, 9, 10]);
    }

    #[test]
    fn rust_does_not_see_calls_inside_macro_arguments() {
        // Tree-sitter keeps a macro's arguments as flat tokens, so this unwrap is invisible.
        let src = "fn f() { assert_eq!(x.unwrap(), 1); }\n";
        let r = Rule { forbid_call: vec!["*.unwrap".into(), "assert_eq!".into()], ..rs_rule() };
        assert_eq!(hits("a.rs", src, &r).len(), 1);
    }

    // ---- Go

    fn go_rule() -> Rule {
        Rule { language: vec![Language::Go], ..empty_rule() }
    }

    #[test]
    fn go_calls_cover_functions_methods_and_builtins() {
        let src = "package main\n\nfunc f() {\n\tfmt.Println(\"hi\")\n\tdb.Query(q).Scan(&x)\n\tpanic(\"no\")\n\tlog.Fatalf(\"x\")\n\tmap_[int](1)\n\tslices.Sort[int](x)\n}\n";
        let r = Rule {
            forbid_call: vec!["fmt.Println".into(), "db.Query().Scan".into(), "panic".into(), "log.Fatal*".into(), "map_".into(), "slices.Sort".into()],
            ..go_rule()
        };
        assert_eq!(lines("a.go", src, &r), [4, 5, 6, 7, 8, 9]);
    }

    #[test]
    fn go_imports_are_read_from_every_spec() {
        let src = "package main\n\nimport \"fmt\"\n\nimport (\n\t\"os\"\n\tdb \"app/internal/db\"\n\t_ \"github.com/lib/pq\"\n)\n";
        let found = imports("main.go", src, &["app/internal/db", "github.com/lib/*", "fmt"], &[]);
        assert_eq!(
            found,
            [
                "importing `fmt` is forbidden",
                "importing `app/internal/db` is forbidden",
                "importing `github.com/lib/pq` is forbidden",
            ]
        );
    }

    fn imports(path: &str, source: &str, forbid: &[&str], except: &[&str]) -> Vec<String> {
        let r = Rule {
            forbid_import: forbid.iter().map(|s| s.to_string()).collect(),
            except_import: except.iter().map(|s| s.to_string()).collect(),
            ..rs_rule()
        };
        hits(path, source, &r).into_iter().map(|h| h.1).collect()
    }

    #[test]
    fn rust_use_declarations_are_expanded() {
        let src = "use std::collections::{HashMap, hash_map::Entry, self};\nuse a::b as c;\npub use d::e::F;\nuse g::*;\nextern crate h;\nuse {i::j, k};\n";
        let all = imports("src/lib.rs", src, &["**"], &[]);
        assert_eq!(
            all,
            [
                "importing `std::collections::HashMap` is forbidden",
                "importing `std::collections::hash_map::Entry` is forbidden",
                "importing `std::collections` is forbidden",
                "importing `a::b` is forbidden",
                "importing `d::e::F` is forbidden",
                "importing `g::*` is forbidden",
                "importing `h` is forbidden",
                "importing `i::j` is forbidden",
                "importing `k` is forbidden",
            ]
        );
    }

    #[test]
    fn rust_patterns_are_written_with_double_colons() {
        let src = "use sqlx::PgPool;\nuse sqlx;\nuse crate::db::Pool;\nuse crate::api::Router;\nuse std::fmt;\n";
        let found = imports("src/domain/user.rs", src, &["sqlx", "sqlx::**", "crate::db::**"], &[]);
        assert_eq!(
            found,
            [
                "importing `sqlx::PgPool` is forbidden",
                "importing `sqlx` is forbidden",
                "importing `crate::db::Pool` is forbidden",
            ]
        );
    }

    #[test]
    fn rust_self_and_super_resolve_to_the_module_path() {
        // src/api/handlers.rs is the module crate::api::handlers.
        let src = "use super::db::Pool;\nuse self::helpers::h;\nuse super::super::config::Settings;\nuse crate::db::Pool as P;\n";
        let found = imports("crate/src/api/handlers.rs", src, &["crate::api::**", "crate::config::**"], &[]);
        assert_eq!(
            found,
            [
                "importing `super::db::Pool` (crate::api::db::Pool) is forbidden",
                "importing `self::helpers::h` (crate::api::handlers::helpers::h) is forbidden",
                "importing `super::super::config::Settings` (crate::config::Settings) is forbidden",
            ]
        );
        // mod.rs and lib.rs are their directory's module, and the crate root.
        let found = imports("src/api/mod.rs", "use self::x::Y;\n", &["crate::api::x::**"], &[]);
        assert_eq!(found, ["importing `self::x::Y` (crate::api::x::Y) is forbidden"]);
        let found = imports("src/lib.rs", "use self::x::Y;\n", &["crate::x::**"], &[]);
        assert_eq!(found, ["importing `self::x::Y` (crate::x::Y) is forbidden"]);
    }

    #[test]
    fn rust_inline_modules_are_part_of_the_module_path() {
        let src = "use super::*;\n#[cfg(test)]\nmod tests {\n    use super::*;\n    use super::helper;\n}\n";
        let found = imports("src/a.rs", src, &["crate::**"], &[]);
        assert_eq!(
            found,
            [
                "importing `super::*` (crate::*) is forbidden", // a top-level module's parent is the crate root
                "importing `super::*` (crate::a::*) is forbidden",
                "importing `super::helper` (crate::a::helper) is forbidden",
            ]
        );
    }

    #[test]
    fn rust_super_above_the_crate_root_is_left_as_written() {
        let found = imports("src/lib.rs", "use super::x;\n", &["super::**"], &[]);
        assert_eq!(found, ["importing `super::x` is forbidden"]);
    }

    #[test]
    fn rust_import_exceptions() {
        let src = "use crate::domain::User;\nuse crate::infra::Db;\nuse std::fmt;\n";
        let found = imports("src/domain/user.rs", src, &["crate::**"], &["crate::domain::**"]);
        assert_eq!(found, ["importing `crate::infra::Db` is forbidden"]);
    }

    // ---- file names

    #[test]
    fn filename_rule_matches_on_the_base_name_only() {
        let rule = Rule { require_filename: vec!["test_*.py".into(), "conftest.py".into()], ..empty_rule() };
        let compiled = CompiledRule::new("r", &rule).unwrap();
        assert!(check_filename("tests/test_users.py", &compiled).is_none());
        assert!(check_filename("tests/unit/conftest.py", &compiled).is_none());
        let hit = check_filename("tests/helpers.py", &compiled).unwrap();
        assert!(hit.message.contains("`helpers.py`"));
        assert!(check_filename("tests/data.json", &compiled).is_some());
        assert!(check_filename("tests/mytest_x.py", &compiled).is_some());
    }
}
