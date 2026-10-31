//! Language adapters: the only place that knows a language's tree-sitter node kinds.
//!
//! Rules are evaluated against a few language-neutral questions ("is this node a raise,
//! and of what?", "is this node a call, and to what?"). Each language answers them from
//! its own syntax tree, so adding a language means adding an adapter, not touching the engine.

mod css;
mod data;
mod ecmascript;
mod go;
mod html;
mod python;
mod rust;
mod shell;

pub use ecmascript::TsConfigs;
pub use go::GoModules;
pub use python::PythonRoots;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use tree_sitter::Node;

use crate::check::Project;
use crate::config::Language;
use crate::data::Doc;

/// One module imported by a statement: as written, and as the project path it leads to.
pub struct ImportRef {
    pub specifier: String,
    pub resolved: String,
}

/// An object-literal property: `key: value`, a shorthand `{ key }`, or a method `key() {}`.
pub struct Property<'t> {
    pub key: String,
    pub value: Option<Node<'t>>,
}

/// A class member that has a name and a place in the file: a field, for example.
pub struct Member<'t> {
    pub name: String,
    pub node: Node<'t>,
}

/// What a class body declares directly (not what it inherits): method names and fields.
pub struct ClassMembers<'t> {
    pub methods: Vec<String>,
    pub fields: Vec<Member<'t>>,
}

pub trait Adapter: Sync {
    /// If `node` raises/throws an exception, the name of what is raised.
    fn raised_name(&self, node: Node, src: &str) -> Option<String>;

    /// If `node` is a call, the callee as written, e.g. `session.exec().one`.
    fn call_name(&self, node: Node, src: &str) -> Option<String>;

    /// If `node` is an exception handler, the names of the types it catches.
    fn caught_types(&self, _node: Node, _src: &str) -> Option<Vec<String>> {
        None
    }

    /// The modules `node` imports. A statement may import several (`use a::{b, c}`).
    fn imports(&self, _node: Node, _src: &str, _file_path: &str, _project: &Project) -> Vec<ImportRef> {
        Vec::new()
    }

    /// If `node` opens a JSX or HTML element, its name, e.g. `button` or `Foo.Bar`.
    fn element_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// If `node` is a property of an object literal, its key and value.
    fn object_property<'t>(&self, _node: Node<'t>, _src: &str) -> Option<Property<'t>> {
        None
    }

    /// If `node` is passed directly as an argument of a call, the callee as written.
    fn argument_of_call(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// Whether `node` is a string literal (a template literal without substitutions counts).
    fn is_string_literal(&self, _node: Node, _src: &str) -> bool {
        false
    }

    /// If `node` is a call, its leading positional arguments, up to the first spread or keyword argument.
    fn call_arguments<'t>(&self, _node: Node<'t>) -> Option<Vec<Node<'t>>> {
        None
    }

    /// The text of a plain string literal (no interpolation), without its quotes.
    fn string_value(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }

    /// If `node` is an array literal, its elements.
    fn array_elements<'t>(&self, _node: Node<'t>) -> Option<Vec<Node<'t>>> {
        None
    }

    /// The class hierarchy this language's classes belong to. Languages that share a family (JavaScript and
    /// TypeScript) can extend each other's classes; others cannot, so a name in one is never a class in another.
    fn class_family(&self) -> Option<&'static str> {
        None
    }

    /// If `node` defines a class, its name and the simple names of its base classes.
    fn class_bases(&self, _node: Node, _src: &str) -> Option<(String, Vec<String>)> {
        None
    }

    /// If `node` defines a class, the methods and fields its body declares.
    fn class_members<'t>(&self, _node: Node<'t>, _src: &str) -> Option<ClassMembers<'t>> {
        None
    }

    /// If `node` defines a function or method, its name. Anonymous functions have none.
    fn function_name(&self, _node: Node, _src: &str) -> Option<String> {
        None
    }
}

pub fn for_language(language: Language) -> &'static dyn Adapter {
    match language {
        Language::Python => &python::Python,
        Language::JavaScript | Language::TypeScript => &ecmascript::EcmaScript,
        Language::Rust => &rust::Rust,
        Language::Go => &go::Go,
        Language::Html => &html::Html,
        Language::Shell => &shell::Shell,
        Language::Css => &css::Css,
        Language::Json | Language::Yaml | Language::Toml => &data::Data,
    }
}

pub fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    &src[node.byte_range()]
}

fn first_named_child(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|n| n.kind() != "comment")
}

/// `./x` or `../x` resolved against the directory of `file_path`; `None` if it leaves the project.
fn relative_path(specifier: &str, file_path: &str) -> Option<String> {
    let mut parts: Vec<&str> = file_path.split('/').collect();
    parts.pop(); // the file name
    for segment in specifier.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// `a/b` joined with `c`; either side may be empty.
fn join(dir: &str, rest: &str) -> String {
    match (dir.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (_, true) => dir.to_string(),
        _ => format!("{dir}/{rest}"),
    }
}

/// What the project's own files say about where imports lead, read once for all adapters:
/// `tsconfig.json` for JavaScript and TypeScript, `go.mod` for Go, `pyproject.toml` for Python.
/// Nothing is executed. Each adapter owns the reading of its files and the use of the result.
#[derive(Debug, Default)]
pub struct Resolvers {
    pub ts: TsConfigs,
    pub go: GoModules,
    pub py: PythonRoots,
}

impl Resolvers {
    /// Reads the files of the languages in `checked`, which are the ones some rule checks imports for.
    /// `paths` is every file in the project; `docs` holds the parsed data files.
    pub fn load(
        root: &Path,
        paths: &[String],
        docs: &BTreeMap<String, Doc>,
        checked: &[Language],
    ) -> Result<Resolvers> {
        let wants = |languages: &[Language]| languages.iter().any(|l| checked.contains(l));
        Ok(Resolvers {
            ts: if wants(&[Language::JavaScript, Language::TypeScript]) { TsConfigs::load(root, paths)? } else { TsConfigs::default() },
            go: if wants(&[Language::Go]) { GoModules::load(root, paths)? } else { GoModules::default() },
            py: if wants(&[Language::Python]) { PythonRoots::load(docs, paths) } else { PythonRoots::default() },
        })
    }

    #[cfg(test)]
    pub fn for_test(go_modules: &[(&str, &str)], files: &[String]) -> Resolvers {
        Resolvers {
            go: GoModules::for_test(go_modules),
            py: PythonRoots::load(&BTreeMap::new(), files),
            ..Default::default()
        }
    }
}
