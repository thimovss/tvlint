use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::{Context, Result};
use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use tree_sitter::Node;

use crate::adapter::{self, Adapter, last_segment};
use crate::config::Rule;
use crate::source::SourceFile;

/// What a check needs to know about the rest of the project.
pub struct Project<'a> {
    pub index: &'a ClassIndex,
    /// Import prefix -> project path, e.g. `@/` -> `src/`.
    pub aliases: &'a BTreeMap<String, String>,
    /// Every file in the project, to find which file an import points at.
    pub paths: &'a HashSet<String>,
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
}

struct CompiledPropertyValue<'a> {
    names: HashSet<&'a str>,
    in_call: GlobSet,
    forbid_literal: bool,
    starts_with: Option<GlobMatcher>,
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
        builder.add(GlobBuilder::new(pattern).literal_separator(true).build()?);
    }
    Ok(builder.build()?)
}

const EXTENSIONS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts", ".json"];

/// The project path an import points at, or the specifier itself for a package.
/// `./x` and `../x` are resolved against the importing file, aliases are expanded, and a path
/// without extension is matched to the file that exists (`x.ts`, `x/index.tsx`, ...).
pub fn resolve_import(specifier: &str, file_path: &str, ctx: &Project) -> String {
    let local = if specifier.starts_with("./") || specifier.starts_with("../") {
        let mut parts: Vec<&str> = file_path.split('/').collect();
        parts.pop(); // the file name
        for segment in specifier.split('/') {
            match segment {
                "." | "" => {}
                ".." => {
                    if parts.pop().is_none() {
                        return specifier.to_string(); // leaves the project
                    }
                }
                other => parts.push(other),
            }
        }
        Some(parts.join("/"))
    } else {
        ctx.aliases
            .iter()
            .filter(|(prefix, _)| specifier.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map(|(prefix, target)| format!("{target}{}", &specifier[prefix.len()..]))
    };
    let Some(path) = local else { return specifier.to_string() };
    let path = path.trim_end_matches('/').to_string();
    if ctx.paths.contains(&path) {
        return path;
    }
    for ext in EXTENSIONS {
        for candidate in [format!("{path}{ext}"), format!("{path}/index{ext}")] {
            if ctx.paths.contains(&candidate) {
                return candidate;
            }
        }
    }
    path
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
        if !rule.import.is_empty()
            && let Some(specifier) = adapter.import_specifier(node, src)
        {
            let resolved = resolve_import(&specifier, &file.path, ctx);
            let hits = |set: &GlobSet| set.is_match(&specifier) || set.is_match(&resolved);
            if hits(&rule.import) && !hits(&rule.except_import) {
                let target = if resolved == specifier { String::new() } else { format!(" ({resolved})") };
                report(node, format!("importing `{specifier}`{target} is forbidden"));
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
        let ctx = Project { index: &index, aliases, paths };
        let mut out = Vec::new();
        check_file(&file, &compiled, &ctx, &mut out);
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
            }),
            ..ts_rule()
        };
        let found = hits("t.ts", src, &r);
        assert_eq!(found.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 4, 6]);
        assert!(found[1].1.contains("an empty array"), "{found:?}");
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
