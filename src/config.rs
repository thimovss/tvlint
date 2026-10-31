use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};

/// The name of a config file. Besides the one at the project root, any directory may hold one.
pub const FILE_NAME: &str = "tvlint.yaml";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rules: BTreeMap<String, Rule>,
    pub scopes: Vec<Scope>,
    /// Import prefixes that stand for a project path, e.g. `"@/": "src/"` for a tsconfig `paths` entry.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    /// Where each rule came from; filled in by `load_tree`.
    #[serde(skip)]
    pub origins: BTreeMap<String, Origin>,
}

/// The config a rule was written in. Its paths are relative to that config's directory, and `load_tree`
/// has already turned the ones that name project files into paths from the project root.
#[derive(Debug, Clone, Default)]
pub struct Origin {
    /// The directory of the config, relative to the project root; empty for the root config.
    pub dir: String,
    /// Import aliases in effect for the rule, as paths from the project root.
    pub aliases: BTreeMap<String, String>,
}

impl Origin {
    /// The config file the rule is written in, from the project root.
    pub fn file(&self) -> String {
        if self.dir.is_empty() { FILE_NAME.to_string() } else { format!("{}/{FILE_NAME}", self.dir) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Rust,
    Go,
    Html,
    Json,
    Yaml,
    Toml,
}

impl Language {
    pub const ALL: &'static [Language] = &[
        Language::Python,
        Language::JavaScript,
        Language::TypeScript,
        Language::Rust,
        Language::Go,
        Language::Html,
        Language::Json,
        Language::Yaml,
        Language::Toml,
    ];

    /// JSON, YAML and TOML: data files, checked by path instead of by syntax.
    pub fn is_data(self) -> bool {
        matches!(self, Language::Json | Language::Yaml | Language::Toml)
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Rust => "rust",
            Language::Go => "go",
            Language::Html => "html",
            Language::Json => "json",
            Language::Yaml => "yaml",
            Language::Toml => "toml",
        }
    }

    pub fn from_name(name: &str) -> Option<Language> {
        Language::ALL.iter().copied().find(|l| l.name() == name)
    }

    /// Whether the language can express the check behind a rule key.
    pub fn supports(self, key: &str) -> bool {
        use Language::*;
        if self.is_data() {
            return matches!(key, "forbid_property" | "property_value" | "require_property" | "same_keys_as" | "same_placeholders");
        }
        if self == Html {
            return matches!(key, "forbid_element" | "forbid_property" | "property_value" | "require_filename");
        }
        match key {
            "forbid_raise" => matches!(self, Python | JavaScript | TypeScript),
            "forbid_catch" => self == Python,
            "forbid_import" | "except_import" => matches!(self, JavaScript | TypeScript | Rust | Go),
            "forbid_element" | "forbid_property" | "property_value" | "within_property" => {
                matches!(self, JavaScript | TypeScript)
            }
            "require_property" | "same_keys_as" | "same_placeholders" => false,
            _ => true,
        }
    }

    /// Why a key is not available, and what to use instead.
    pub fn unsupported_hint(self, key: &str) -> &'static str {
        if self.is_data() {
            return "JSON, YAML and TOML files are checked by key path: use forbid_property, require_property, property_value, same_keys_as";
        }
        match (self, key) {
            (Language::Html, _) => "HTML supports forbid_element (tag names), forbid_property (attribute names) and property_value (attribute values)",
            (_, "require_property" | "same_keys_as" | "same_placeholders") => "these checks read JSON, YAML and TOML files",
            (Language::Rust, "forbid_raise") => {
                "Rust has no exceptions; use forbid_call with panic!, unreachable!, \"*.unwrap\" or \"*.expect\""
            }
            (Language::Go, "forbid_raise") => "Go has no exceptions; use forbid_call with panic or \"log.Fatal*\"",
            (_, "forbid_catch") => "only Python `except` clauses name the type they catch",
            (Language::Python, "forbid_import" | "except_import") => "import rules are not supported for Python yet",
            (_, "forbid_import" | "except_import") => "import rules are not supported for this language",
            _ => "JSX elements, object literals and within_property exist only in JavaScript and TypeScript",
        }
    }

    pub fn from_path(path: &Path) -> Option<Language> {
        match path.extension()?.to_str()? {
            "py" => Some(Language::Python),
            "rs" => Some(Language::Rust),
            "go" => Some(Language::Go),
            "html" | "htm" => Some(Language::Html),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
            "json" | "jsonc" => Some(Language::Json),
            "yaml" | "yml" => Some(Language::Yaml),
            "toml" => Some(Language::Toml),
            _ => None,
        }
    }
}

/// Accepts `names: queryKey` as well as `names: [queryKey, other]`.
fn string_or_list<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(item) => vec![item],
        OneOrMany::Many(items) => items,
    })
}

/// Accepts `language: python` as well as `language: [javascript, typescript]`.
fn one_or_many<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Language>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(Language),
        Many(Vec<Language>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(language) => vec![language],
        OneOrMany::Many(languages) => languages,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    /// Which files the rule applies to. Required for code checks; empty means every
    /// file, which only makes sense for file-name rules.
    #[serde(default, deserialize_with = "one_or_many")]
    pub language: Vec<Language>,
    #[serde(default)]
    pub forbid_raise: Vec<String>,
    #[serde(default)]
    pub forbid_call: Vec<String>,
    pub forbid_catch: Option<ClassSelector>,
    /// Module specifiers that may not be imported. Relative and aliased imports are matched
    /// by their project path, anything else by the specifier as written.
    #[serde(default)]
    pub forbid_import: Vec<String>,
    /// Imports that are fine even though they match `forbid_import`.
    #[serde(default)]
    pub except_import: Vec<String>,
    /// JSX element names that may not be used, e.g. `button`.
    #[serde(default)]
    pub forbid_element: Vec<String>,
    /// Object-literal property names that may not appear.
    #[serde(default)]
    pub forbid_property: Vec<String>,
    /// Constraints on the value of object-literal properties.
    pub property_value: Option<PropertyValue>,
    /// Only count `forbid_call`, `forbid_raise` and `forbid_element` hits that sit inside
    /// an object-literal property with one of these names, e.g. `onError`.
    #[serde(default)]
    pub within_property: Vec<String>,
    /// Key paths that must exist, e.g. `jobs.*.timeout-minutes`: every match of the parent path must have the key.
    #[serde(default)]
    pub require_property: Vec<String>,
    /// A file (relative to the config) whose key paths every matched data file must have too.
    pub same_keys_as: Option<String>,
    /// Also require the `{{placeholders}}` of each shared message to be the same as in `same_keys_as`.
    #[serde(default)]
    pub same_placeholders: bool,
    /// String literals passed to calls must be listed in a data file.
    pub literal_argument: Option<LiteralArgument>,
    /// Every matched file's name must match one of these globs.
    #[serde(default)]
    pub require_filename: Vec<String>,
    pub rationale: String,
    pub fix: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyValue {
    #[serde(deserialize_with = "string_or_list")]
    pub names: Vec<String>,
    /// Only look at objects passed directly as an argument to one of these calls.
    #[serde(default, deserialize_with = "string_or_list")]
    pub in_call: Vec<String>,
    /// The value must not be a string literal.
    #[serde(default)]
    pub forbid_literal: bool,
    /// An array-literal value must start with this element (matched as a glob on its source text).
    pub array_starts_with: Option<String>,
    /// Data files: the value must not match any of these globs, e.g. `^*` or `*:latest`.
    #[serde(default, deserialize_with = "string_or_list")]
    pub forbid_value: Vec<String>,
    /// Data files: the value must match one of these globs.
    #[serde(default, deserialize_with = "string_or_list")]
    pub require_value: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiteralArgument {
    /// Calls whose argument is checked, as patterns like `forbid_call`.
    #[serde(deserialize_with = "string_or_list")]
    pub calls: Vec<String>,
    /// Which argument, counting from 0.
    #[serde(default)]
    pub position: usize,
    pub registry: Registry,
    /// Also report registry entries that no call in scope uses.
    #[serde(default)]
    pub report_unused: bool,
}

/// What a registry entry is called: the matched key, the matched scalar, or the scalar's full path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Use {
    Key,
    Value,
    Path,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    /// The data file listing the allowed values, relative to the config.
    pub file: String,
    /// Which nodes of the file are entries.
    pub path: String,
    /// What an entry is called: its `key`, its `value` (default) or its full `path`.
    #[serde(default = "default_use", rename = "use")]
    pub use_: Use,
}

fn default_use() -> Use {
    Use::Value
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassSelector {
    pub subclass_of: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub rules: Vec<String>,
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl Rule {
    pub fn has_code_checks(&self) -> bool {
        !self.code_keys().is_empty()
    }

    /// The code checks this rule uses, by rule key.
    pub fn code_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();
        for (key, present) in [
            ("forbid_raise", !self.forbid_raise.is_empty()),
            ("forbid_call", !self.forbid_call.is_empty()),
            ("forbid_catch", self.forbid_catch.is_some()),
            ("forbid_import", !self.forbid_import.is_empty()),
            ("except_import", !self.except_import.is_empty()),
            ("forbid_element", !self.forbid_element.is_empty()),
            ("forbid_property", !self.forbid_property.is_empty()),
            ("property_value", self.property_value.is_some()),
            ("within_property", !self.within_property.is_empty()),
            ("require_property", !self.require_property.is_empty()),
            ("same_keys_as", self.same_keys_as.is_some()),
            ("same_placeholders", self.same_placeholders),
            ("literal_argument", self.literal_argument.is_some()),
        ] {
            if present {
                keys.push(key);
            }
        }
        keys
    }

    /// The first check that one of the rule's languages cannot express.
    pub fn unsupported(&self) -> Option<(&'static str, Language)> {
        self.code_keys()
            .into_iter()
            .find_map(|key| self.language.iter().find(|l| !l.supports(key)).map(|l| (key, *l)))
    }

    /// One line per check, for `explain` and `validate`.
    pub fn describe_checks(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.forbid_raise.is_empty() {
            out.push(format!("raising {}", self.forbid_raise.join(", ")));
        }
        if !self.forbid_call.is_empty() {
            out.push(format!("calling {}", self.forbid_call.join(", ")));
        }
        if let Some(catch) = &self.forbid_catch {
            out.push(format!("catching {} or its subclasses", catch.subclass_of));
        }
        if !self.forbid_import.is_empty() {
            let except = if self.except_import.is_empty() {
                String::new()
            } else {
                format!(" (except {})", self.except_import.join(", "))
            };
            out.push(format!("importing {}{except}", self.forbid_import.join(", ")));
        }
        if !self.forbid_element.is_empty() {
            let tags: Vec<String> = self.forbid_element.iter().map(|e| format!("<{e}>")).collect();
            out.push(format!("using {}", tags.join(", ")));
        }
        if !self.forbid_property.is_empty() {
            out.push(format!("object properties named {}", self.forbid_property.join(", ")));
        }
        if let Some(pv) = &self.property_value {
            let mut parts = Vec::new();
            if !pv.forbid_value.is_empty() {
                parts.push(format!("the value {}", pv.forbid_value.join(", ")));
            }
            if !pv.require_value.is_empty() {
                parts.push(format!("any value but {}", pv.require_value.join(", ")));
            }
            if pv.forbid_literal {
                parts.push("string literals".to_string());
            }
            if let Some(start) = &pv.array_starts_with {
                parts.push(format!("arrays not starting with {start}"));
            }
            let context = if pv.in_call.is_empty() { String::new() } else { format!(" in {}(...)", pv.in_call.join("/")) };
            out.push(format!("{} as the value of {}{context}", parts.join(" or "), pv.names.join(", ")));
        }
        if !self.require_property.is_empty() {
            out.push(format!("files without the key {}", self.require_property.join(", ")));
        }
        if let Some(reference) = &self.same_keys_as {
            let what = if self.same_placeholders { "keys and {{placeholders}}" } else { "keys" };
            out.push(format!("files whose {what} differ from {reference}"));
        }
        if let Some(arg) = &self.literal_argument {
            let unused = if arg.report_unused { ", and entries nothing uses" } else { "" };
            out.push(format!(
                "argument {} of {} that is not in {}{unused}",
                arg.position,
                arg.calls.join("/"),
                arg.registry.file
            ));
        }
        if !self.within_property.is_empty() {
            out.push(format!("(only inside a property named {})", self.within_property.join(", ")));
        }
        if !self.require_filename.is_empty() {
            out.push(format!("file names other than {}", self.require_filename.join(", ")));
        }
        out
    }

    pub fn applies_to(&self, language: Language) -> bool {
        self.language.is_empty() || self.language.contains(&language)
    }

    pub fn applies_to_path(&self, path: &str) -> bool {
        self.language.is_empty()
            || Language::from_path(Path::new(path)).is_some_and(|language| self.language.contains(&language))
    }

    /// `python`, `javascript/typescript`, or `any language`.
    pub fn language_label(&self) -> String {
        if self.language.is_empty() {
            "any language".to_string()
        } else {
            self.language.iter().map(|l| l.name()).collect::<Vec<_>>().join("/")
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Config::parse(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    /// Loads the config at `path` together with the nested configs `nested` (paths from its directory, parents
    /// before children). A nested config adds rules that apply to its own directory only, and its paths are
    /// relative to that directory.
    pub fn load_tree(path: &Path, nested: &[String]) -> Result<Config> {
        let mut config = Config::load(path)?;
        let own = std::path::absolute(path)?;
        let root = own.parent().context("config file has no parent directory")?.to_path_buf();
        config.origins = config
            .rules
            .keys()
            .map(|id| (id.clone(), Origin { dir: String::new(), aliases: config.aliases.clone() }))
            .collect();
        // The aliases in effect in each directory, for nested configs to inherit; parents come before children.
        let mut in_effect: Vec<(String, BTreeMap<String, String>)> = vec![(String::new(), config.aliases.clone())];
        for file in nested {
            let dir = file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
            let mut nested = Config::load(&root.join(file))?;
            nested.rebase(&dir);
            let mut aliases = in_effect
                .iter()
                .filter(|(d, _)| d.is_empty() || dir == *d || dir.starts_with(&format!("{d}/")))
                .max_by_key(|(d, _)| d.len())
                .map(|(_, aliases)| aliases.clone())
                .unwrap_or_default();
            aliases.extend(nested.aliases);
            // A nested rule is known by its directory and its id (`web/no-print`), so ids only
            // have to be unique within one config.
            let global = |id: &str| format!("{dir}/{id}");
            for (id, rule) in nested.rules {
                let id = global(&id);
                if let Some(first) = config.origins.get(&id) {
                    bail!("rule `{id}` is defined in {} and in {file}", first.file());
                }
                config.origins.insert(id.clone(), Origin { dir: dir.clone(), aliases: aliases.clone() });
                config.rules.insert(id, rule);
            }
            for mut scope in nested.scopes {
                scope.rules = scope.rules.iter().map(|id| global(id)).collect();
                config.scopes.push(scope);
            }
            in_effect.push((dir, aliases));
        }
        Ok(config)
    }

    /// Turns every path that is relative to the config's directory `dir` into one from the project root.
    fn rebase(&mut self, dir: &str) {
        for scope in &mut self.scopes {
            for pattern in scope.include.iter_mut().chain(&mut scope.exclude) {
                *pattern = format!("{dir}/{}", pattern.trim_start_matches("./"));
            }
        }
        for rule in self.rules.values_mut() {
            if let Some(file) = &mut rule.same_keys_as {
                *file = join(dir, file);
            }
            if let Some(arg) = &mut rule.literal_argument {
                arg.registry.file = join(dir, &arg.registry.file);
            }
        }
        for target in self.aliases.values_mut() {
            *target = join(dir, target);
        }
    }

    pub fn parse(text: &str) -> Result<Config> {
        let config: Config = serde_yaml_ng::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        for (id, rule) in &self.rules {
            if let Some(problem) = id_problem(id) {
                bail!("rule id `{id}` {problem}");
            }
            if !rule.has_code_checks() && rule.require_filename.is_empty() {
                bail!("rule `{id}` has no checks (expected forbid_raise, forbid_call, forbid_catch, forbid_import, forbid_element, forbid_property, property_value, require_property, same_keys_as, literal_argument or require_filename)");
            }
            if rule.has_code_checks() && rule.language.is_empty() {
                bail!("rule `{id}` needs `language` because it has code checks (forbid_raise, forbid_call, forbid_catch, forbid_import, ...)");
            }
            if let Some((key, language)) = rule.unsupported() {
                bail!("rule `{id}`: {key} is not supported for {} ({})", language.name(), language.unsupported_hint(key));
            }
            if !rule.except_import.is_empty() && rule.forbid_import.is_empty() {
                bail!("rule `{id}`: except_import needs forbid_import");
            }
            if !rule.within_property.is_empty()
                && rule.forbid_call.is_empty()
                && rule.forbid_raise.is_empty()
                && rule.forbid_element.is_empty()
            {
                bail!("rule `{id}`: within_property needs forbid_call, forbid_raise or forbid_element");
            }
            let data = rule.language.iter().any(|l| l.is_data());
            if data && rule.language.iter().any(|l| !l.is_data()) {
                bail!("rule `{id}`: JSON, YAML and TOML cannot share a rule with code languages; make one rule for each");
            }
            if let Some(pv) = &rule.property_value {
                if data {
                    if pv.forbid_literal || pv.array_starts_with.is_some() || !pv.in_call.is_empty() {
                        bail!("rule `{id}`: for data files property_value takes `names`, `forbid_value` and `require_value` (not forbid_literal, array_starts_with or in_call)");
                    }
                    if pv.forbid_value.is_empty() && pv.require_value.is_empty() {
                        bail!("rule `{id}`: property_value needs forbid_value or require_value");
                    }
                } else {
                    if !pv.forbid_value.is_empty() || !pv.require_value.is_empty() {
                        bail!("rule `{id}`: forbid_value and require_value read JSON, YAML and TOML; for code use forbid_literal or array_starts_with");
                    }
                    if !pv.forbid_literal && pv.array_starts_with.is_none() {
                        bail!("rule `{id}`: property_value needs forbid_literal or array_starts_with");
                    }
                }
            }
            if rule.same_placeholders && rule.same_keys_as.is_none() {
                bail!("rule `{id}`: same_placeholders needs same_keys_as");
            }
            if let Some(arg) = &rule.literal_argument
                && arg.calls.is_empty()
            {
                bail!("rule `{id}`: literal_argument needs at least one call in `calls`");
            }
        }
        for scope in &self.scopes {
            for id in &scope.rules {
                if !self.rules.contains_key(id) {
                    bail!("scope references unknown rule `{id}`");
                }
            }
        }
        Ok(())
    }
}

/// Why `id` is not a valid rule id: lowercase words joined by single dashes, like `no-direct-db-access`.
/// A `/` is out so that `web/no-print`, the name of a nested rule, can only mean one rule.
pub fn id_problem(id: &str) -> Option<&'static str> {
    let valid = !id.is_empty() && id.split('-').all(|word| !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase()));
    (!valid).then_some("must be lowercase words joined by dashes, like `no-direct-db-access` (no slashes, digits, underscores or capitals)")
}

/// `path` inside the directory `dir`, as a path from the project root; `..` and `.` are resolved and a trailing `/` is kept.
fn join(dir: &str, path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in dir.split('/').chain(path.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    let slash = if path.ends_with('/') { "/" } else { "" };
    format!("{}{slash}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_ids_are_lowercase_words_joined_by_dashes() {
        for good in ["a", "no-print", "no-direct-db-access"] {
            assert!(id_problem(good).is_none(), "{good}");
        }
        for bad in ["", "web/no-print", "No-Print", "no_print", "-a", "a-", "a--b", "no-md5", "a b"] {
            assert!(id_problem(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn join_resolves_paths_against_the_config_directory() {
        assert_eq!(join("web", "src/"), "web/src/");
        assert_eq!(join("a/b", "./codes.json"), "a/b/codes.json");
        assert_eq!(join("a/b", "../shared/codes.json"), "a/shared/codes.json");
    }
}
