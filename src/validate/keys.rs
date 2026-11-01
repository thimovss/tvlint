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
    ("forbid_import", "Python, JS/TS, Rust, Go, shell (`source`/`.`) and CSS (`@import`); module specifiers that must not be imported; relative, aliased and in-project imports match by project path; `{1}`, `{2}`... stand for what the scope's include wildcards matched"),
    ("except_import", "Python, JS/TS, Rust, Go, shell and CSS; imports that are fine although they match forbid_import; may use the same `{1}` placeholders"),
    ("forbid_element", "JS/TS and HTML; JSX element or HTML tag names that must not be used, e.g. `button`"),
    ("forbid_property", "JS/TS: object-literal property names that must not appear. HTML: attribute names. CSS: declaration names such as `float`. JSON/YAML/TOML: key paths that must not exist, e.g. `scripts.postinstall`, `jobs.*.permissions`, `**.debug`"),
    ("property_value", "JS/TS and HTML (attributes): `names`, plus `forbid_literal` and/or `array_starts_with`, and optionally `in_call`. JSON/YAML/TOML: `names` (key paths) plus `forbid_value` and/or `require_value` (globs on the value)"),
    ("require_property", "JSON/YAML/TOML; key paths that must exist, e.g. `jobs.*.timeout-minutes`: every match of the parent path needs the key"),
    ("same_keys_as", "JSON/YAML/TOML; path of a reference file whose keys every matched file must have, no more and no fewer"),
    ("same_placeholders", "JSON/YAML/TOML; true: shared messages must also use the same `{{placeholders}}` as the same_keys_as file"),
    ("literal_argument", "Python, JS/TS, Rust, Go, shell, CSS; `calls`, `position` and a `registry` (file, path, use), `pattern`, `require_literal`, `lookup`: string literals passed to those calls must be listed in a data file and/or have a shape; `report_unused: true` also reports entries nothing uses"),
    ("within_property", "JS/TS; only count call, raise and element hits inside a property with one of these names"),
    ("max_lines", "any language; a positive number: files with more lines are reported, from the first line over the limit"),
    ("forbid_assignment", "Python, JS/TS; target globs, as written (`aggregate.*`, `self.items[]`), that may not be assigned, `+=` included"),
    ("forbid_augmented_assignment", "Python, JS/TS; target globs that may not be changed with `+=`, `-=` and the like"),
    ("forbid_literal", "Python, JS/TS; `compared: true` forbids a string literal in ==, !=, === and !==; `assigned_to: [globs]` forbids assigning one to matching names"),
    ("require_argument", "Python; `calls` and `names`: each matching call must pass these keyword arguments (skipped when the call has **kwargs)"),
    ("forbid_argument", "Python; `calls` and `names`: matching calls must not pass these keyword arguments"),
    ("argument_keys", "Python, JS/TS; `calls`, `position` (or `keyword`) and `forbid_keys` / `allow_keys`: the keys of a dict or object literal passed to a call"),
    ("allow_raise", "Python, JS/TS; `names` and/or `subclass_of`: only these exceptions may be raised (a lowercase name is a variable and is left alone)"),
    ("literal_assignment", "Python, JS/TS; `names` and a `registry`, `pattern`, `require_literal`: string literals assigned to these names must be listed in a data file and/or have a shape"),
    ("max_functions", "Python, JS/TS; `max`, and optionally `names` and `containing_element` (JS/TS): at most that many top-level functions of that kind per file, e.g. one React component"),
    ("forbid_export", "JS/TS; `names` (default any), `declared: true` (only `export const Foo`, `export default function Foo`) and `default: true|false`: exports that must not exist"),
    ("require_export", "JS/TS; `names` and `default: true`: every file needs an export like this, e.g. `export default Foo;`"),
    ("require_call", "Python, JS/TS, Rust, Go, shell; call patterns, one of which must appear in each file, or in each function that within_function selects"),
    ("require_method", "Python, JS/TS; method name globs that every selected class must declare itself (inherited ones do not count)"),
    ("require_field", "Python, JS/TS; field name globs that every selected class must declare itself: annotated or assigned names in the class body"),
    ("forbid_field", "Python, JS/TS; field name globs that no selected class may declare"),
    ("in_class", "Python, JS/TS; which classes the class checks look at: `subclass_of: Name` (not Name itself) and/or `names` (globs); default every class"),
    ("within_function", "Python, JS/TS; function name globs: forbid_call and forbid_raise only count inside them, and require_call applies to each of them instead of the file"),
    ("except_function", "Python, JS/TS; function name globs that within_function leaves out, e.g. `_*`"),
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
