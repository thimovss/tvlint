# TVLint

Enforce your team's code rules deterministically. Rules live in a YAML file, violations come back with a reason and a fix so both humans and AI agents can act on them.

Supports Python, JavaScript, TypeScript and Rust.

## Install

```bash
cargo install --path .
```

## Usage

In a project without a config, `tvl init` writes an empty `tvl.yaml` to fill in. Then run from the directory that contains `tvl.yaml`:

```bash
tvl
```

```
tvl [OPTIONS] [COMMAND]
```

### Commands

| Command | What it does |
|---|---|
| `check [PATH]...` | Checks the project against all rules and prints every violation. This is the default when no command is given, so `tvl` and `tvl check` are the same. Pass files, directories or glob patterns to check only those (see below). |
| `init [--force]` | Scaffolds an empty `tvl.yaml`: no rules, only a commented example and the commands to run next. Refuses to overwrite an existing file unless `--force` is given. |
| `list` | Lists all configured rules with their language and the number of scopes that use them. |
| `explain <rule>` | Shows why a rule exists, the fix, what it forbids, and which paths it applies to. |
| `validate` | Checks the config file and explains every problem it finds, with the path to the key and how to fix it. Also tells you how many files each rule will run on. Use it after writing or changing a rule. |
| `schema` | Prints the JSON Schema of the config file (also kept in [tvl.schema.json](tvl.schema.json)). Point your editor at it for completion and inline errors, or give it to an AI agent that writes rules. |
| `help [command]` | Prints help for TVLint or for one command. |

### Checking specific files

`tvl check` takes any number of files, directories or glob patterns, relative to the directory you run it from. Without any, it checks the whole project.

```bash
tvl check src/api.ts                    # one file
tvl check src/routes src/hooks          # directories: everything below them
tvl check "src/**/*.test.ts"            # a glob: quote it so tvl expands it, not the shell
tvl check -c frontend/tvl.yaml frontend/src/App.tsx   # from the repo root, as a pre-commit hook does
```

- A file is still only checked by the rules whose scope includes it. Passing a path narrows what runs, it never widens a scope.
- Paths must be inside the project, the directory of the config file, and must exist, otherwise TVLint exits with `2`. A glob that matches nothing is a warning on stderr. Files ignored by `.gitignore` are skipped, as always.
- Everything is still parsed, so rules that follow imports or class hierarchies see the same project as in a full run, but only the selected files are reported on.
- With `--staged`, a file must be both staged and selected.

### Flags

| Flag | What it does |
|---|---|
| `--staged` | `check` only. Checks only the files staged in git, using their staged contents, so unstaged edits are ignored. Meant for pre-commit hooks. Needs a git repository (exit `2` otherwise). Only staged files under the project root are considered. Combined with `PATH` arguments, a file must be both staged and selected. |
| `--format <FORMAT>` | `check` only. `text` (default) is for people. `json` prints one document with `violations` (each with `rule`, `file`, `line`, `column`, `message`, `rationale` and `fix`) and a `summary`, for scripts and AI agents. `sarif` prints SARIF 2.1.0 for GitHub code scanning and editors; the `fix` goes in the rule's `help` and the result message. With `json` or `sarif`, stdout holds only the document (warnings go to stderr), and the exit code is the same: `1` when there are violations. |
| `-c, --config <path>` | Path to the config file. Defaults to `tvl.yaml` in the current directory. The project root is the directory containing this file, and scope paths are relative to it. Works with every command. |
| `-h, --help` | Prints help. |
| `-V, --version` | Prints the version. |

### Validating rules

`tvl validate` is meant for people and AI agents that add rules. It reports all problems in one pass, not just the first.

- **Errors** make the config invalid (exit `1`): unknown or misspelled keys (with a "did you mean" suggestion), missing `language`, `rationale` or `fix`, wrong types, a rule with no checks, invalid globs, scopes that name a rule that doesn't exist, bad YAML syntax.
- **Warnings** don't fail validation (exit `0`) but usually mean a mistake: a rule no scope uses, a scope that matches no files for the rule's language, a `subclass_of` class that isn't defined in the project. They are only reported once there are no errors.
- When valid, it lists each rule with what it forbids and how many files it runs on. A rule that runs on `0 file(s)` is almost always a wrong glob.

### Editor and agent support

`tvl.schema.json` describes the config file: keys, types, which languages each check supports, and which keys need each other. Add this as the first line of `tvl.yaml` and editors with the YAML language server (VS Code, JetBrains, Neovim) complete keys and flag mistakes as you type:

```yaml
# yaml-language-server: $schema=./tvl.schema.json
```

`tvl schema` prints the same schema, so it works from an installed binary: `tvl schema > tvl.schema.json`. The schema cannot check that scopes name existing rules, that globs are valid, or that a scope matches any files; `tvl validate` does.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | No violations. |
| `1` | At least one violation was found (`check`), or the config is invalid (`validate`). |
| `2` | TVLint could not run: missing or invalid config, unknown rule, bad glob. The message goes to stderr. |

Violations go to stdout, one per rule hit:

```
backend/src/auth/router/save_attribute.py:39:13: error[no-http-exceptions] raising `ValueError` is forbidden
    why: One global handler produces every error body. Ad-hoc responses bypass the code system.
    fix: Raise a ExampleException subclass. A new HTTP status needs a new subclass, not an inline response.
```

## Languages

| Language | Files |
|---|---|
| `python` | `.py` |
| `javascript` | `.js`, `.jsx`, `.mjs`, `.cjs` |
| `typescript` | `.ts`, `.tsx`, `.mts`, `.cts` |
| `rust` | `.rs` |
| `json` | `.json`, `.jsonc` |
| `yaml` | `.yaml`, `.yml` |
| `toml` | `.toml` |

Rules are written once and apply to every language they name, for example `language: [javascript, typescript]` for a rule on a mixed codebase. Each language has an adapter that maps its syntax tree to the same few questions the rules ask (what is thrown, what is called, what is imported). Not every language can express every check: Rust has no exceptions, so it has no `forbid_raise` or `forbid_catch`, and only JavaScript and TypeScript have JSX and object-literal checks. `tvl validate` says so when a rule asks a language for something it cannot do.

JSON, YAML and TOML are data, not code: they are checked by key path (see [Data files](#data-files)), and a rule cannot mix them with code languages. Hidden directories such as `.github/` are read; `.git` and anything in `.gitignore` are not.

### Output for tools

```bash
tvl check --format json > tvl-report.json
tvl check --format sarif > tvl.sarif
```

To show violations as annotations on pull requests, upload the SARIF file with GitHub's `github/codeql-action/upload-sarif` step.

## Suppressing violations

Two comments switch checks off. They work the same in Python, JavaScript and TypeScript, and are read from real comments, so the same words inside a string do nothing.

| Comment | Effect |
|---|---|
| `tvlint-ignore-next-line` | Hides every violation reported on the line directly after the comment. |
| `tvlint-ignore-file` | On line 1 of the file, excludes the whole file from every rule, file-name rules included. |

```python
# tvlint-ignore-next-line: legacy endpoint, see ISSUE-123
raise ValueError("not found")
```

```tsx
{/* tvlint-ignore-next-line */}
<button onClick={open}>Open</button>
```

```ts
// tvlint-ignore-file: generated code
```

- Use the comment style of the language: `#` in Python; `//` or `/* */` in JavaScript and TypeScript, and `{/* */}` inside JSX.
- The directive must start the comment. A reason may follow it (`: why`), and is a good idea, but is not required.
- **Next line** means the line right after the comment's last line. A violation is reported on the line where the flagged code starts, so for a statement spanning several lines, put the comment above its first line. A trailing comment (`code  # tvlint-ignore-next-line`) hides the next line, not its own.
- **File** needs the comment to be the first thing on line 1. A shebang line or a blank line before it means it does not count. Only Python, JavaScript and TypeScript files can carry it.
- An ignored file is still read to learn its classes, so other files' `forbid_catch` results do not change when a file opts out.
- Nothing is silent: the summary line says how many violations were hidden and how many files were skipped, e.g. `tvl: 7 violation(s) in 4 file(s) [5 hidden by tvlint-ignore-next-line, 3 file(s) skipped by tvlint-ignore-file]`.

## Data files

JSON, YAML and TOML files are read into one model of maps, lists and values, so the same rule works whichever format a file uses. Rules address a value by **key path**, anchored at the root of the file:

| Path | Matches |
|---|---|
| `scripts.postinstall` | the `postinstall` key inside `scripts` |
| `jobs.*.runs-on` | `runs-on` in every job. `*` is any one key **or list index** |
| `jobs.*.steps.*.uses` | `uses` in every step of every job (list items are addressed by index) |
| `**.version` | `version` at any depth, including the top level |
| `dependencies.@types/*` | a `*` or `?` inside a segment is a glob on that key |
| `"a.b".c` | a key that contains a dot goes in double quotes |

- **`forbid_property`** reports the key where it is written.
- **`require_property`** splits the path into the container and the key: `jobs.*.timeout-minutes` means "every job has `timeout-minutes`". A container that does not exist (`permissions.contents` when there is no `permissions`) is reported once for the file; a wildcard container that matches nothing is fine. The last segment must be a key name. In a multi-document YAML file every document is checked.
- **`property_value`** looks at the scalar values at the `names` paths, with `forbid_value: [globs]` (none may match) and/or `require_value: [globs]` (one must match). Values are plain text, so `true`, `"true"` and `1.0` are just the text of the value, and quotes and escapes are removed. Lists and mappings are skipped. Values are globs, where `*` matches anything: write a literal star as `[*]` (`forbid_value: ["[*]"]` forbids a wildcard version).
- **`same_keys_as`** compares the keys of every in-scope file with a reference file (usually the source-language translation): `a.b` must exist in both. Lists count as values. Add `same_placeholders: true` to also compare the `{{name}}`, `{name}` and ICU `{count, plural, ...}` names in each message.
- Comments work as in code: in YAML and TOML, `# tvlint-ignore-next-line` hides the violations of the next line, and `# tvlint-ignore-file` on line 1 skips the file. JSON has no comment syntax of its own, but the parser accepts `//` and `/* */` comments, so a directive works in `.json` and `.jsonc` files alike (tools that read the file strictly will reject them).
- YAML aliases (`*name`) and merge keys (`<<`) are not resolved; an alias is a value whose text is `*name`.

```yaml
rules:
  workflow-jobs-have-timeouts:
    language: yaml
    require_property: ["jobs.*.timeout-minutes"]
    rationale: A hung job runs for six hours.
    fix: Add timeout-minutes to the job.

  pinned-npm-versions:
    language: json
    property_value:
      names: ["dependencies.*", "devDependencies.*"]
      forbid_value: ["^*", "~*", latest]
    rationale: Ranges make installs differ between machines.
    fix: Pin the exact version.

  locale-files-match:
    language: json
    same_keys_as: src/locales/en.json
    same_placeholders: true
    rationale: A missing key renders as a raw key in the UI.
    fix: Add the missing keys, translated.

scopes:
  - rules: [workflow-jobs-have-timeouts]
    include: [".github/workflows/*.yml"]
  - rules: [pinned-npm-versions]
    include: ["package.json"]
  - rules: [locale-files-match]
    include: ["src/locales/*.json"]
```

### Code that must match a data file

`literal_argument` ties code to a data file: string literals passed to certain calls must be entries of a registry.

```yaml
rules:
  error-codes-are-registered:
    language: python
    literal_argument:
      calls: ["*Exception"]            # patterns, as in forbid_call
      position: 0                      # which argument, counting from 0 (default 0)
      registry:
        file: backend/error_codes.json # relative to the config
        path: "codes.*"                # key path selecting the entries
        use: value                     # what an entry is called: value (default), key or path
      report_unused: true              # also report entries that no call uses
    rationale: The frontend translates errors by code.
    fix: Add the code to backend/error_codes.json.
```

- `use: value` takes the scalar values at `path`, `use: key` their keys, and `use: path` the full dotted path of each scalar, so `t("menu.home")` matches `{"menu": {"home": "Home"}}` with `path: "**"`.
- Only plain string literals are checked: an argument that is a variable, an f-string or a template with `${}` is skipped, as is a call with a spread before the argument.
- `report_unused` reports each entry that no call in the rule's scope passes, in the registry file, at the entry. With `--staged` or `PATH` arguments the usage is still collected from the whole scope, and the entries are reported only if the registry file is selected.
- The registry file must exist and be JSON, YAML or TOML (`validate` checks, and warns when `path` selects nothing).

## Config

Looking for rules to start from? See the [cookbook](cookbook/).

`tvl.yaml` has two sections: `rules` says what is forbidden, `scopes` says which files each rule runs on.

```yaml
rules:
  no-http-exceptions:
    language: python
    forbid_raise: [HTTPException, ValueError]
    forbid_catch: { subclass_of: ExampleException }
    rationale: One global handler produces every error body.
    fix: Raise a ExampleException subclass.

scopes:
  - rules: [no-http-exceptions]
    include: ["backend/src/*/router/**"]
    exclude: ["backend/src/tests/**"]
```

### Rule keys

| Key | Meaning |
|---|---|
| `language` | `python`, `javascript`, `typescript`, `rust`, `json`, `yaml` or `toml`, or a list such as `[javascript, typescript]`. Limits the rule to files of those languages. Required with every check except `require_filename`. Optional with `require_filename`, where omitting it applies the rule to every file type. |
| `forbid_raise` | Python, JavaScript and TypeScript. Exception names that may not be raised (`throw` in JavaScript and TypeScript, where `throw new Foo()` names `Foo`). Matches the full dotted name or its last part, so `HTTPException` also matches `fastapi.HTTPException`. Bare `raise` (re-raise) is allowed. |
| `forbid_call` | Call patterns that are forbidden. `*` matches anything, so `*.one` matches `session.exec(q).one()` and `console.*` matches `console.log(...)`. In JavaScript and TypeScript, `new Foo()` counts as a call to `Foo`. In Rust, functions are written with `::` (`std::fs::read`, `Vec::new`), methods with `.` (`*.unwrap` matches `s.parse::<i32>().unwrap()`), and macros with their `!` (`println!`); calls inside a macro's arguments are not seen. Imports and aliases are not resolved. |
| `forbid_catch` | Python only, because JavaScript and TypeScript `catch` clauses have no exception type. `subclass_of: Name` forbids `except` clauses that catch `Name` or any class inheriting from it. Subclasses are found by class name across the whole project, not by import path. |
| `forbid_import` | JavaScript, TypeScript and Rust. Module specifiers that may not be imported (`import`, `export ... from`, `import()`, `require()`; in Rust `use` and `extern crate`, where `use a::{b, c}` counts as `a::b` and `a::c`, and `self::` and `super::` are resolved to `crate::` paths). Relative imports and imports through an alias are matched by their project path (`src/hooks/use-mobile.ts`), anything else by the specifier as written (`@base-ui/react/dialog`). Patterns are path globs: `*` stays within one segment, `**` crosses segments, so list a package and its subpaths (`react-aria-components`, `react-aria-components/**`). Rust patterns are written with `::` (`sqlx::**`, `crate::db::**`). |
| `except_import` | Imports that are fine although they match `forbid_import`. Together they express "this folder may only import from here": `forbid_import: ["src/**"]` with `except_import: ["src/components/ui/**"]`. |
| `forbid_element` | JavaScript and TypeScript. JSX element names that may not be used, e.g. `[button, textarea]`. Matched on the tag as written, so `button` does not match `Button`. |
| `forbid_property` | JavaScript and TypeScript: object-literal property names that may not appear (`{ onError: ... }`). JSX attributes such as `<img onError={...}>` are not properties. JSON, YAML and TOML: key paths that may not exist, e.g. `scripts.postinstall` ([key paths](#data-files)). |
| `property_value` | JavaScript and TypeScript: constraints on the value of object-literal properties: `names` (required), then `forbid_literal: true` (the value may not be a string literal) and/or `array_starts_with: accountID` (an array-literal value must start with that element; other values are not checked). `in_call: toast.add` limits it to objects passed directly to that call. JSON, YAML and TOML: `names` are key paths, with `forbid_value` and/or `require_value` (globs, see [Data files](#data-files)). |
| `require_property` | JSON, YAML and TOML. Key paths that must exist, e.g. `jobs.*.timeout-minutes`: every place the parent path (`jobs.*`) matches must contain the key. |
| `same_keys_as` | JSON, YAML and TOML. Path to a reference file. Every file in the rule's scope must have exactly its keys: missing and extra keys are reported. The reference itself is skipped. |
| `same_placeholders` | JSON, YAML and TOML. `true` also requires each shared message to use the same `{{name}}` / `{name}` placeholders as the reference. Needs `same_keys_as`. |
| `literal_argument` | Python, JavaScript, TypeScript and Rust. String literals passed to calls must be listed in a data file, see [Code that must match a data file](#code-that-must-match-a-data-file). |
| `within_property` | JavaScript and TypeScript. Narrows `forbid_call`, `forbid_raise` and `forbid_element` to hits that sit inside an object-literal property with one of these names, e.g. `onError`. |
| `require_filename` | List of file name globs, matched against the file's base name only (not its directory). Every file in the rule's scope must match at least one, otherwise it is a violation reported at line 1. Applies to all files in scope, not just parseable ones, so `data.json` or `helpers.py` in a `tests/` scope are flagged too. |
| `rationale` | Required. Why the rule exists. Printed with every violation. |
| `fix` | Required. How to comply. Printed with every violation. |

A rule needs at least one of `forbid_raise`, `forbid_call`, `forbid_catch` or `require_filename`. Unknown keys are rejected.

### Top-level keys

| Key | Meaning |
|---|---|
| `rules` | Required. The rules, by id. |
| `scopes` | Required. Which files each rule runs on. |
| `aliases` | Optional. Import prefixes that stand for a project path, such as the `paths` entry of a `tsconfig.json`: `"@/": "src/"` turns `@/api` into `src/api`. Needed for `forbid_import` rules to see where an aliased import leads. The prefix and the path must both end with `/`, or neither. |

### Scope keys

| Key | Meaning |
|---|---|
| `rules` | Rule ids that run on the matched files. |
| `include` | Glob patterns relative to the project root. `*` stays within one directory, `**` crosses directories. |
| `exclude` | Optional globs to leave out. |

A rule that appears in no scope never runs. Files ignored by `.gitignore` are skipped.

## License

AGPL-3.0-only. See [LICENSE](LICENSE). To contribute, see [CONTRIBUTING.md](CONTRIBUTING.md); contributors sign a [CLA](CLA.md) so the maintainer can also offer TVLint under other terms.
