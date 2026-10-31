//! The keys and languages the config accepts, with the one-line help shown for each.

pub(super) const TOP_KEYS: &[(&str, &str)] = &[
    ("rules", "mapping of rule id to rule definition"),
    ("scopes", "list of entries that say which files each rule runs on"),
    ("aliases", "optional; import prefix to project path, e.g. `\"@/\": \"src/\"`"),
];

pub(super) const RULE_KEYS: &[(&str, &str)] = &[
    ("language", "`python`, `javascript`, `typescript`, `rust`, `go`, `html`, `shell`, `css`, `json`, `yaml` or `toml`, or a list of them; required with every check except require_filename. Data languages cannot be mixed with code languages"),
    ("forbid_raise", "Python, JS/TS; list of exception names that must not be raised (`throw` in JavaScript and TypeScript)"),
    ("forbid_call", "list of call patterns that must not appear; `*` matches anything, e.g. `*.one`; Rust macros end in `!`; Go calls look like `fmt.Println`; in shell a call is a command name such as `curl`; in CSS a function such as `rgb`"),
    ("forbid_catch", "python only; `subclass_of: Name` forbids catching Name or any class inheriting from it"),
    ("forbid_import", "Python, JS/TS, Rust, Go, shell (`source`/`.`) and CSS (`@import`); module specifiers that must not be imported; relative, aliased and in-project imports match by project path"),
    ("except_import", "Python, JS/TS, Rust, Go, shell and CSS; imports that are fine although they match forbid_import"),
    ("forbid_element", "JS/TS and HTML; JSX element or HTML tag names that must not be used, e.g. `button`"),
    ("forbid_property", "JS/TS: object-literal property names that must not appear. HTML: attribute names. CSS: declaration names such as `float`. JSON/YAML/TOML: key paths that must not exist, e.g. `scripts.postinstall`, `jobs.*.permissions`, `**.debug`"),
    ("property_value", "JS/TS and HTML (attributes): `names`, plus `forbid_literal` and/or `array_starts_with`, and optionally `in_call`. JSON/YAML/TOML: `names` (key paths) plus `forbid_value` and/or `require_value` (globs on the value)"),
    ("require_property", "JSON/YAML/TOML; key paths that must exist, e.g. `jobs.*.timeout-minutes`: every match of the parent path needs the key"),
    ("same_keys_as", "JSON/YAML/TOML; path of a reference file whose keys every matched file must have, no more and no fewer"),
    ("same_placeholders", "JSON/YAML/TOML; true: shared messages must also use the same `{{placeholders}}` as the same_keys_as file"),
    ("literal_argument", "Python, JS/TS, Rust, Go, shell, CSS; `calls`, `position` and a `registry` (file, path, use): string literals passed to those calls must be listed in a data file; `report_unused: true` also reports entries nothing uses"),
    ("within_property", "JS/TS; only count call, raise and element hits inside a property with one of these names"),
    ("max_lines", "any language; a positive number: files with more lines are reported, from the first line over the limit"),
    ("require_filename", "list of file name globs; any matched file whose name matches none is a violation"),
    ("rationale", "required; why the rule exists, shown with every violation"),
    ("fix", "required; how to comply, shown with every violation"),
];

pub(super) const SCOPE_KEYS: &[(&str, &str)] = &[
    ("rules", "required; list of rule ids that run on the matched files"),
    ("include", "required; list of path globs relative to the config file's directory"),
    ("exclude", "optional; list of path globs to leave out"),
];

pub(super) const LANGUAGES: &[&str] = &["python", "javascript", "typescript", "rust", "go", "html", "shell", "css", "json", "yaml", "toml"];
