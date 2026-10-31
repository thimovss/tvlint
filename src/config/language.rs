//! The languages TVLint reads, and what each can express.

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Rust,
    Go,
    Html,
    Shell,
    Css,
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
        Language::Shell,
        Language::Css,
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
            Language::Shell => "shell",
            Language::Css => "css",
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
            return matches!(key, "forbid_property" | "property_value" | "require_property" | "same_keys_as" | "same_placeholders" | "max_lines");
        }
        if self == Css {
            return matches!(key, "forbid_call" | "forbid_import" | "except_import" | "forbid_property" | "literal_argument" | "max_lines" | "require_filename");
        }
        if self == Html {
            return matches!(key, "forbid_element" | "forbid_property" | "property_value" | "max_lines" | "require_filename");
        }
        match key {
            "forbid_raise" => matches!(self, Python | JavaScript | TypeScript),
            "forbid_catch" => self == Python,
            "forbid_import" | "except_import" => matches!(self, Python | JavaScript | TypeScript | Rust | Go | Shell),
            "forbid_element" | "forbid_property" | "property_value" | "within_property" => {
                matches!(self, JavaScript | TypeScript)
            }
            "require_property" | "same_keys_as" | "same_placeholders" => false,
            "require_method" | "require_field" | "forbid_field" | "in_class" | "within_function" | "except_function" => {
                matches!(self, Python | JavaScript | TypeScript)
            }
            _ => true,
        }
    }

    /// Why a key is not available, and what to use instead.
    pub fn unsupported_hint(self, key: &str) -> &'static str {
        if self.is_data() {
            return "JSON, YAML and TOML files are checked by key path: use forbid_property, require_property, property_value, same_keys_as";
        }
        match (self, key) {
            (Language::Css, _) => "CSS supports forbid_call (functions such as `rgb`), forbid_import (`@import`) and forbid_property (declaration names)",
            (Language::Html, _) => "HTML supports forbid_element (tag names), forbid_property (attribute names) and property_value (attribute values)",
            (_, "require_property" | "same_keys_as" | "same_placeholders") => "these checks read JSON, YAML and TOML files",
            (_, "require_method" | "require_field" | "forbid_field" | "in_class") => "class checks exist for Python, JavaScript and TypeScript",
            (_, "within_function" | "except_function") => "function checks exist for Python, JavaScript and TypeScript",
            (Language::Rust, "forbid_raise") => {
                "Rust has no exceptions; use forbid_call with panic!, unreachable!, \"*.unwrap\" or \"*.expect\""
            }
            (Language::Shell, "forbid_raise") => "shell has no exceptions; use forbid_call with the command, e.g. `exit` or `set`",
            (Language::Go, "forbid_raise") => "Go has no exceptions; use forbid_call with panic or \"log.Fatal*\"",
            (_, "forbid_catch") => "only Python `except` clauses name the type they catch",
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
            "sh" | "bash" => Some(Language::Shell),
            "css" => Some(Language::Css),
            "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
            "json" | "jsonc" => Some(Language::Json),
            "yaml" | "yml" => Some(Language::Yaml),
            "toml" => Some(Language::Toml),
            _ => None,
        }
    }
}
