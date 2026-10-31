//! The config file: rules, the scopes that select their files, and nested configs.

mod captures;
mod language;
mod rule;

pub use captures::{CapturePattern, fill as fill_placeholders};

pub use language::Language;
pub use rule::{ArgumentCheck, ClassFilter, Lookup, Rule, Use};
#[cfg(test)]
pub use rule::{AllowRaise, ArgumentKeys, ForbidLiteral};
#[cfg(test)]
pub use rule::{ClassSelector, LiteralArgument, PropertyValue, Registry};

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub rules: Vec<String>,
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
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
            for arg in [&mut rule.literal_argument, &mut rule.literal_assignment].into_iter().flatten() {
                if let Some(registry) = &mut arg.registry {
                    registry.file = join(dir, &registry.file);
                }
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
                bail!("rule `{id}` has no checks (expected forbid_raise, forbid_call, forbid_catch, forbid_import, forbid_element, forbid_property, property_value, require_property, same_keys_as, literal_argument, literal_assignment, max_lines, require_call, require_method, require_field, forbid_field, forbid_assignment, forbid_augmented_assignment, forbid_literal, require_argument, forbid_argument, argument_keys, allow_raise or require_filename)");
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
            let class_checks = !rule.require_method.is_empty() || !rule.require_field.is_empty() || !rule.forbid_field.is_empty();
            if let Some(filter) = &rule.in_class {
                if !class_checks {
                    bail!("rule `{id}`: in_class needs require_method, require_field or forbid_field");
                }
                if filter.subclass_of.is_none() && filter.names.is_empty() {
                    bail!("rule `{id}`: in_class needs subclass_of or names (leave it out to check every class)");
                }
            }
            if !rule.within_function.is_empty() && !rule.narrowable_by_function() {
                bail!("rule `{id}`: within_function narrows forbid_call, forbid_raise, require_call, assignment, literal, argument and allow_raise checks, and the rule has none");
            }
            if !rule.except_function.is_empty() && rule.within_function.is_empty() {
                bail!("rule `{id}`: except_function needs within_function (use `within_function: [\"*\"]` for every function)");
            }
            if rule.same_placeholders && rule.same_keys_as.is_none() {
                bail!("rule `{id}`: same_placeholders needs same_keys_as");
            }
            if rule.literal_argument.is_some() && rule.literal_assignment.is_some() {
                bail!("rule `{id}`: literal_argument and literal_assignment cannot share a rule; make one rule for each");
            }
            if let Some(arg) = &rule.literal_argument {
                if arg.calls.is_empty() {
                    bail!("rule `{id}`: literal_argument needs at least one call in `calls`");
                }
                if !arg.names.is_empty() {
                    bail!("rule `{id}`: literal_argument watches `calls`; `names` belongs to literal_assignment");
                }
            }
            if let Some(arg) = &rule.literal_assignment {
                if arg.names.is_empty() {
                    bail!("rule `{id}`: literal_assignment needs at least one name in `names`");
                }
                if !arg.calls.is_empty() || arg.position != 0 {
                    bail!("rule `{id}`: literal_assignment watches `names`; `calls` and `position` belong to literal_argument");
                }
            }
            if let Some(arg) = rule.literal_check() {
                if arg.registry.is_none() && arg.pattern.is_empty() && !arg.require_literal {
                    bail!("rule `{id}`: the literal check has nothing to check; give it a `registry`, a `pattern` or `require_literal: true`");
                }
                if arg.registry.is_none() && (arg.report_unused || arg.lookup != Lookup::Value) {
                    bail!("rule `{id}`: report_unused and lookup need a registry");
                }
            }
            if let Some(lit) = &rule.forbid_literal
                && !lit.compared
                && lit.assigned_to.is_empty()
            {
                bail!("rule `{id}`: forbid_literal needs `compared: true` or `assigned_to`");
            }
            for (key, check) in [("require_argument", &rule.require_argument), ("forbid_argument", &rule.forbid_argument)] {
                if let Some(check) = check
                    && (check.calls.is_empty() || check.names.is_empty())
                {
                    bail!("rule `{id}`: {key} needs `calls` and `names`");
                }
            }
            if let Some(keys) = &rule.argument_keys
                && (keys.calls.is_empty() || (keys.forbid_keys.is_empty() && keys.allow_keys.is_empty()))
            {
                bail!("rule `{id}`: argument_keys needs `calls` and `forbid_keys` or `allow_keys`");
            }
            if let Some(allow) = &rule.allow_raise
                && allow.names.is_empty()
                && allow.subclass_of.is_none()
            {
                bail!("rule `{id}`: allow_raise needs `names` or `subclass_of`");
            }
        }
        for scope in &self.scopes {
            for id in &scope.rules {
                let Some(rule) = self.rules.get(id) else { bail!("scope references unknown rule `{id}`") };
                let needed = rule.max_placeholder();
                for include in scope.include.iter().filter(|_| needed > 0) {
                    let wildcards = CapturePattern::new(include)?.wildcards();
                    if wildcards < needed {
                        bail!("rule `{id}` uses the placeholder {{{needed}}} in its import patterns, but the scope include `{include}` has only {wildcards} wildcard segment(s) to capture; each `*` segment of an include fills one placeholder, in order");
                    }
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
mod tests;
