use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::Language;
use crate::source;

/// A cookbook recipe the starter config is assembled from. `suffix` keeps ids unique when the
/// cookbook has the same rule for several languages.
struct Recipe {
    id: &'static str,
    suffix: &'static str,
    text: &'static str,
}

/// What the project contains, which decides the starter rules.
#[derive(Default)]
struct Detected {
    python: bool,
    script: bool,
    rust: bool,
    cargo_toml: bool,
    package_json: bool,
}

impl Detected {
    fn scan(paths: &[String]) -> Detected {
        let mut found = Detected::default();
        for path in paths {
            match Language::from_path(Path::new(path)) {
                Some(Language::Python) => found.python = true,
                Some(Language::JavaScript | Language::TypeScript) => found.script = true,
                Some(Language::Rust) => found.rust = true,
                _ => {}
            }
            found.cargo_toml |= path == "Cargo.toml";
            found.package_json |= path == "package.json";
        }
        found
    }

    fn recipes(&self) -> Vec<Recipe> {
        let mut recipes = Vec::new();
        if self.python {
            recipes.push(Recipe { id: "no-wall-clock-in-tests", suffix: "python", text: include_str!("../cookbook/python/no-wall-clock-in-tests.yaml") });
            recipes.push(Recipe { id: "no-randomness-in-tests", suffix: "python", text: include_str!("../cookbook/python/no-randomness-in-tests.yaml") });
        }
        if self.script {
            recipes.push(Recipe { id: "no-wall-clock-in-tests", suffix: "js-ts", text: include_str!("../cookbook/typescript/no-wall-clock-in-tests.yaml") });
            recipes.push(Recipe { id: "no-randomness-in-tests", suffix: "js-ts", text: include_str!("../cookbook/typescript/no-randomness-in-tests.yaml") });
        }
        if self.rust {
            recipes.push(Recipe { id: "no-wall-clock-in-tests", suffix: "rust", text: include_str!("../cookbook/rust/no-wall-clock-in-tests.yaml") });
            recipes.push(Recipe { id: "no-randomness-in-tests", suffix: "rust", text: include_str!("../cookbook/rust/no-randomness-in-tests.yaml") });
        }
        if self.cargo_toml {
            recipes.push(Recipe { id: "no-wildcard-dependency-versions", suffix: "", text: include_str!("../cookbook/toml/no-wildcard-dependency-versions.yaml") });
        }
        if self.package_json {
            recipes.push(Recipe { id: "pinned-npm-versions", suffix: "", text: include_str!("../cookbook/json/pinned-npm-versions.yaml") });
        }
        recipes
    }
}

const HEADER: &str = "\
# prlint.yaml: the rules of this project, checked deterministically.
#
#   prlint validate        is this file correct?
#   prlint check           does the code follow it?
#   prlint explain <rule>  why does a rule exist, and how do I comply?
#
# Every rule carries a rationale and a fix, which prlint prints with each violation so a
# developer or an AI agent knows why it exists and how to comply. `scopes` say which files a
# rule applies to.
";

/// The part of a recipe between `marker` and `until` (or its end), without the marker line itself.
fn section<'a>(text: &'a str, marker: &str, until: Option<&str>) -> &'a str {
    let start = text.find(marker).map_or(text.len(), |i| i + marker.len());
    let rest = &text[start..];
    let end = until.and_then(|u| rest.find(u)).unwrap_or(rest.len());
    rest[..end].trim_matches('\n')
}

/// Builds the config text for a project containing `paths` (relative to its root).
fn generate(paths: &[String]) -> (String, Vec<String>) {
    let recipes = Detected::scan(paths).recipes();
    let mut out = String::from(HEADER);
    if recipes.is_empty() {
        return (String::new(), Vec::new());
    }
    let renamed = |recipe: &Recipe, text: &str| match recipe.suffix {
        "" => text.to_string(),
        suffix => text.replace(recipe.id, &format!("{}-{suffix}", recipe.id)),
    };
    let rules: Vec<String> = recipes.iter().map(|r| renamed(r, section(r.text, "\nrules:\n", Some("\nscopes:\n")))).collect();
    let scopes: Vec<String> = recipes.iter().map(|r| renamed(r, section(r.text, "\nscopes:\n", None))).collect();
    out.push_str("\nrules:\n");
    out.push_str(&rules.join("\n\n"));
    out.push_str("\n\nscopes:\n");
    out.push_str(&scopes.join("\n"));
    out.push('\n');
    let ids = recipes
        .iter()
        .map(|r| if r.suffix.is_empty() { r.id.to_string() } else { format!("{}-{}", r.id, r.suffix) })
        .collect();
    (out, ids)
}

/// Writes a starter config for the project at `config_path`'s directory. Never overwrites an
/// existing file unless `force` is set.
pub fn run(config_path: &Path, force: bool) -> Result<()> {
    if config_path.exists() && !force {
        bail!("{} already exists; pass --force to overwrite it", config_path.display());
    }
    let root = std::path::absolute(config_path)?
        .parent()
        .map(Path::to_path_buf)
        .context("config file has no parent directory")?;
    let (text, ids) = generate(&source::list_all(&root)?);
    if ids.is_empty() {
        bail!("found no Python, JavaScript, TypeScript or Rust files to choose starter rules for; add some code, or write {} by hand (`prlint schema` prints its JSON Schema)", config_path.display());
    }
    std::fs::write(config_path, text).with_context(|| format!("cannot write {}", config_path.display()))?;

    println!("Created {}", config_path.display());
    println!("Starter rules, taken from the cookbook:");
    for id in ids {
        println!("  {id}");
    }
    println!("\nNext: prlint validate, then prlint check. Edit the file to add your own rules.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn every_project_shape_gives_a_valid_config() {
        let shapes: &[&[&str]] = &[
            &["a.py"],
            &["web/a.ts", "web/b.jsx"],
            &["src/main.rs", "Cargo.toml"],
            &["a.py", "web/a.ts", "src/main.rs", "Cargo.toml", "package.json"],
        ];
        for shape in shapes {
            let (text, ids) = generate(&paths(shape));
            let config = Config::parse(&text).unwrap_or_else(|e| panic!("{shape:?}: {e:#}\n{text}"));
            assert_eq!(config.rules.len(), ids.len(), "{shape:?}");
            // Every scope names a rule that exists.
            for scope in &config.scopes {
                for rule in &scope.rules {
                    assert!(config.rules.contains_key(rule), "{shape:?}: scope names unknown rule {rule}");
                }
            }
        }
    }

    #[test]
    fn same_rule_for_several_languages_gets_distinct_ids() {
        let (text, _) = generate(&paths(&["a.py", "b.ts", "c.rs"]));
        let config = Config::parse(&text).unwrap();
        for id in ["no-wall-clock-in-tests-python", "no-wall-clock-in-tests-js-ts", "no-wall-clock-in-tests-rust"] {
            assert!(config.rules.contains_key(id), "{id} missing");
        }
    }

    #[test]
    fn only_matching_manifests_add_rules() {
        let (text, _) = generate(&paths(&["Cargo.toml"]));
        assert!(Config::parse(&text).unwrap().rules.contains_key("no-wildcard-dependency-versions"));
        let (text, _) = generate(&paths(&["package.json"]));
        assert!(Config::parse(&text).unwrap().rules.contains_key("pinned-npm-versions"));
        // Nested manifests are not the root one.
        let (_, ids) = generate(&paths(&["sub/Cargo.toml"]));
        assert!(ids.is_empty());
    }
}
