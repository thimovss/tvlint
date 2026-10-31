# <img src="icon.svg" alt="" width="36" height="36" align="absmiddle"> TVLint

[![TVLint](https://github.com/thimovss/tvlint-poc/actions/workflows/tvlint.yml/badge.svg)](https://github.com/thimovss/tvlint-poc/actions/workflows/tvlint.yml)

Enforce your team's code rules deterministically. Rules live in a YAML file, violations come back with a reason and a fix so both humans and AI agents can act on them.

Supports Python, JavaScript, TypeScript, Rust, Go, HTML, shell and CSS, plus JSON, YAML and TOML data files.

## Install

```bash
cargo install --path .
```

## Usage

In a project without a config, `tvl init` writes an empty `tvlint.yaml` to fill in. Then run from the directory that contains `tvlint.yaml`:

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
| `init [--force]` | Scaffolds an empty `tvlint.yaml`: no rules, only a commented example and the commands to run next. Refuses to overwrite an existing file unless `--force` is given. |
| `list` | Lists all configured rules with their language and the number of scopes that use them. |
| `explain <rule>` | Shows why a rule exists, the fix, what it forbids, and which paths it applies to. |
| `validate` | Checks the config file and explains every problem it finds, with the path to the key and how to fix it. Also tells you how many files each rule will run on. Use it after writing or changing a rule. |
| `schema` | Prints the JSON Schema of the config file (also kept in [tvlint.schema.json](tvlint.schema.json)). Point your editor at it for completion and inline errors, or give it to an AI agent that writes rules. |
| `help [command]` | Prints help for TVLint or for one command. |

### Checking specific files

`tvl check` takes any number of files, directories or glob patterns, relative to the directory you run it from. Without any, it checks the whole project.

```bash
tvl check src/api.ts                    # one file
tvl check src/routes src/hooks          # directories: everything below them
tvl check "src/**/*.test.ts"            # a glob: quote it so tvl expands it, not the shell
tvl check -c frontend/tvlint.yaml frontend/src/App.tsx   # from the repo root, as a pre-commit hook does
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
| `-c, --config <path>` | Path to the config file. Defaults to `tvlint.yaml` in the current directory. The project root is the directory containing this file, and scope paths are relative to it. Works with every command. |
| `-h, --help` | Prints help. |
| `-V, --version` | Prints the version. |

### Validating rules

`tvl validate` is meant for people and AI agents that add rules. It reports all problems in one pass, not just the first.

- **Errors** make the config invalid (exit `1`): unknown or misspelled keys (with a "did you mean" suggestion), missing `language`, `rationale` or `fix`, wrong types, a rule with no checks, invalid globs, scopes that name a rule that doesn't exist, bad YAML syntax.
- **Warnings** don't fail validation (exit `0`) but usually mean a mistake: a rule no scope uses, a scope that matches no files for the rule's language, a `subclass_of` class that isn't defined in the project. They are only reported once there are no errors.
- When valid, it lists each rule with what it forbids and how many files it runs on. A rule that runs on `0 file(s)` is almost always a wrong glob.

### Editor and agent support

`tvlint.schema.json` describes the config file: keys, types, which languages each check supports, and which keys need each other. Add this as the first line of `tvlint.yaml` and editors with the YAML language server (VS Code, JetBrains, Neovim) complete keys and flag mistakes as you type:

```yaml
# yaml-language-server: $schema=./tvlint.schema.json
```

`tvl schema` prints the same schema, so it works from an installed binary: `tvl schema > tvlint.schema.json`. The schema cannot check that scopes name existing rules, that globs are valid, or that a scope matches any files; `tvl validate` does.

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
| `go` | `.go` |
| `html` | `.html`, `.htm` |
| `shell` | `.sh`, `.bash` |
| `css` | `.css` |
| `json` | `.json`, `.jsonc` |
| `yaml` | `.yaml`, `.yml` |
| `toml` | `.toml` |

Rules are written once and apply to every language they name, for example `language: [javascript, typescript]` for a rule on a mixed codebase. Each language has an adapter that maps its syntax tree to the same few questions the rules ask (what is thrown, what is called, what is imported). Not every language can express every check: Rust and Go have no exceptions, so they have no `forbid_raise` or `forbid_catch`, and only JavaScript and TypeScript have object-literal checks. HTML is markup, so it has no calls or imports: it supports `forbid_element` (tag names, case-insensitive), `forbid_property` (attribute names) and `property_value` with `forbid_literal` (attributes that carry a value), plus `require_filename`. Code inside `<script>` and `<style>` is not parsed. Shell scripts have commands and `source`: `forbid_call` matches the command name as written (`curl`, `sudo`, `./deploy.sh`; a name built from an expansion such as `$CMD` is `?`), `forbid_import` matches the file given to `source` or `.`, and `literal_argument` reads a command's words. Only bare and quoted words without expansions have a value; shell has no `forbid_raise`. CSS supports `forbid_call` (functions in values, lowercased: `rgb`, `calc`, `url`), `forbid_import` (`@import`; a bare name is relative to the stylesheet, a URL stays as written) and `forbid_property` (declaration names such as `float`; custom properties keep their case). Selectors, `!important` and declaration values are not checked. `tvl validate` says so when a rule asks a language for something it cannot do.

JSON, YAML and TOML are data, not code: they are checked by key path (see [Data files](#data-files)), and a rule cannot mix them with code languages. Hidden directories such as `.github/` are read; `.git` and anything in `.gitignore` are not.

### Output for tools

```bash
tvl check --format json > tvl-report.json
tvl check --format sarif > tvl.sarif
```

To show violations as annotations on pull requests, upload the SARIF file with GitHub's `github/codeql-action/upload-sarif` step.

### Running in GitHub Actions

A job that runs `tvl check` fails when there are violations (exit code 1), so the workflow is green or red on its own:

```yaml
name: TVLint
on:
  push:
    branches: [main]
  pull_request:
jobs:
  tvl:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: tvl check   # install tvl first, see Install
```

### Badge

GitHub draws a pass/fail badge for any workflow, so there is nothing to host. Put this in your README, replacing `OWNER/REPO` and the file name (`tvlint.yml`) with your own:

```markdown
[![TVLint](https://github.com/OWNER/REPO/actions/workflows/tvlint.yml/badge.svg)](https://github.com/OWNER/REPO/actions/workflows/tvlint.yml)
```

It shows the state of the latest run on the default branch. Add `?branch=main` to the image URL to pin a branch, or `?event=push` to ignore pull request runs.

### Import aliases

For JavaScript and TypeScript, `forbid_import` matches an aliased import by the project path it leads to. TVLint reads the aliases from the files your compiler already uses, without running `tsc` or Node:

- The nearest `tsconfig.json` (or `jsconfig.json`) above the importing file decides, as in TypeScript. If both are in one directory, `tsconfig.json` is used.
- `compilerOptions.paths` and `baseUrl` are read, with `extends` followed (relative files, and packages in `node_modules`). A child's `paths` replaces its parent's. A pattern with several targets uses the first one that exists.
- A config whose `paths` live in a referenced file (`"references": [{ "path": "./tsconfig.app.json" }]`, as in Vite projects) also gets the referenced file's aliases.
- Comments and trailing commas are fine.
- A config that cannot be read, an `extends` that does not exist or loops, or invalid JSON stops `tvl check` with exit code 2 and names the file. This is only checked when a rule has `forbid_import` for JavaScript or TypeScript.
- Aliases written in `tvlint.yaml` (`aliases`) win over the tsconfig ones. Bundler-only aliases (Vite, webpack, Jest) live in JavaScript and cannot be read, so list them there.

### Imports in Python and Go

Python and Go imports are matched by the project path they lead to, using the files those ecosystems already keep. Nothing is run.

- **Go:** `go.mod` gives the module path. With `module github.com/acme/app`, the import `github.com/acme/app/internal/db` resolves to the directory `internal/db` (a nested `go.mod` wins for its own directory). Packages are directories, so list both `internal/db` and `internal/db/**`. Standard library and third-party imports stay as written (`fmt`, `github.com/lib/pq`). A `go.mod` without a `module` line stops `tvl check` with exit code 2.
- **Python:** `import a.b as c` is `a.b`, and `from a.b import c, d` is `a.b.c` and `a.b.d`, as with Rust's `use a::{b, c}`; `from a import *` is `a.*`. List a package and its names (`sqlalchemy`, `sqlalchemy.**`). Patterns may use the dotted name (`app.db.**`) or the project path it resolves to (`src/app/db/**`). Absolute imports start at the source roots of the nearest `pyproject.toml`: `tool.setuptools.packages.find.where`, `tool.setuptools.package-dir`, `tool.poetry.packages[].from`, `tool.pytest.ini_options.pythonpath` and the folder of `tool.hatch.build.targets.wheel.packages`, plus the directory of the `pyproject.toml` and its `src/` folder. Relative imports (`from . import x`, `from ..p import y`) start at the importing file. `a.b.c` also finds `a/b.py` when `c` is a name inside it. Anything that does not lead to a project file is a third-party package and stays as written.

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

- Use the comment style of the language: `#` in Python and shell; `//` or `/* */` in JavaScript and TypeScript, and `{/* */}` inside JSX; `<!-- -->` in HTML; `/* */` in CSS.
- The directive must start the comment. A reason may follow it (`: why`), and is a good idea, but is not required.
- **Next line** means the line right after the comment's last line. A violation is reported on the line where the flagged code starts, so for a statement spanning several lines, put the comment above its first line. A trailing comment (`code  # tvlint-ignore-next-line`) hides the next line, not its own.
- **File** needs the comment to be the first thing on line 1. A shebang line or a blank line before it means it does not count. Every language can carry it, but a shell script's shebang already takes line 1, so use `tvlint-ignore-next-line` there.
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

## Classes and functions

Some rules are about what a class declares or what a function does, not about what a file imports. Python, JavaScript and TypeScript can express them; the rule reads the same in each.

```yaml
rules:
  events-declare-apply-and-describe:
    language: python
    require_method: [apply, describe]      # a glob: "{apply,create}" accepts either
    forbid_field: [author, account_id]     # envelope data does not belong on the event
    in_class: { subclass_of: DomainEvent } # DomainEvent[Agg] counts; DomainEvent itself does not
    rationale: Events are replayed forever and shown in the history list.
    fix: Declare apply() and describe() on the event, and keep envelope data on the Event row.

  commands-emit-events:
    language: python
    require_call: [apply_event, "*.apply_event", create_aggregate]
    within_function: ["*"]                 # every function in scope ...
    except_function: ["_*"]                # ... except the private ones
    rationale: A command changes data by emitting an event.
    fix: End the command with apply_event or create_aggregate.

  no-clock-in-apply:
    language: python
    forbid_call: ["date.today", "datetime.now"]
    within_function: [apply]               # only hits inside apply() count
    rationale: apply() replays old events and must give the same result every time.
    fix: Read the clock in the command and put the value on the event.
```

- **Classes** are found by name across the project, like `forbid_catch`. `subclass_of` follows inheritance through the project's own classes, and Python and TypeScript keep separate hierarchies (JavaScript and TypeScript share one). A base written with type arguments (`Base[T]`, `Base<T>`) is still `Base`.
- **Members are what the class body declares itself.** A method inherited from a base class does not satisfy `require_method`, so a base class that defines `apply` does not hide a subclass that forgot to.
- **Fields** are annotated or assigned names in the class body (`x: int`, `x = 1`). Instance attributes set in `__init__` are not fields.
- **Functions** are `def`s and methods, and in JavaScript and TypeScript also `const f = () => ...`. Anonymous functions are not named, so a hit inside one belongs to the function around it.
- `require_call` counts calls anywhere in the function body, nested blocks and nested functions included. It does not follow calls into other functions.
- A class is reported on its `class` line and a function on its `def` line, so `tvlint-ignore-next-line` goes above that line.

## Placeholders in import patterns

`forbid_import` and `except_import` may use `{1}`, `{2}`, ... for what the wildcard segments of the scope's `include` pattern matched, in order. This lets one rule say "a package may import itself" instead of one scope for every package:

```yaml
rules:
  subscriptions-use-their-own-commands:
    language: python
    forbid_import: ["src/*/commands/**"]
    except_import: ["src/{1}/commands/**"]    # {1} is the package in the include pattern
    rationale: A subscription lives with the aggregate that reacts to the event.
    fix: Move it to the package of the aggregate it acts on.

scopes:
  - rules: [subscriptions-use-their-own-commands]
    include: ["src/*/subscriptions/*.py"]     # the first * fills {1}
```

For `src/assets/subscriptions/media_links.py`, `{1}` is `assets`, so importing `src/assets/commands/**` is fine and `src/contracts/commands/**` is reported.

- Each segment of the `include` pattern that has a `*`, `?`, `[..]` or `{..}` in it fills one placeholder, with the whole path segment it matched (`*.py` captures the file name, extension included). `**` captures nothing.
- A scope with several `include` patterns uses the first one that matches the file. If a rule is in several scopes, the first scope that selects the file counts.
- A rule that uses `{2}` in a scope whose `include` has only one wildcard segment is an error when the config loads.
- Only whole numbers in braces are placeholders, so `{a,b}` alternation still works.

## Config

Looking for rules to start from? See the [cookbook](cookbook/).

`tvlint.yaml` has two sections: `rules` says what is forbidden, `scopes` says which files each rule runs on.

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
| `forbid_import` | Python, JavaScript, TypeScript, Rust, Go, shell and CSS. Module specifiers that may not be imported (`import`, `export ... from`, `import()`, `require()`; in Rust `use` and `extern crate`, where `use a::{b, c}` counts as `a::b` and `a::c`, and `self::` and `super::` are resolved to `crate::` paths). Relative imports and imports through an alias are matched by their project path (`src/hooks/use-mobile.ts`), anything else by the specifier as written (`@base-ui/react/dialog`). Patterns are path globs: `*` stays within one segment, `**` crosses segments, so list a package and its subpaths (`react-aria-components`, `react-aria-components/**`). Rust patterns are written with `::` (`sqlx::**`, `crate::db::**`). |
| `except_import` | Imports that are fine although they match `forbid_import`. Together they express "this folder may only import from here": `forbid_import: ["src/**"]` with `except_import: ["src/components/ui/**"]`. Both take [placeholders](#placeholders-in-import-patterns) such as `{1}`. |
| `forbid_element` | JavaScript, TypeScript and HTML. JSX element or HTML tag names that may not be used, e.g. `[button, textarea]`. JSX is matched as written, so `button` does not match `Button`; HTML tag and attribute names are lowercased first. |
| `forbid_property` | JavaScript and TypeScript: object-literal property names that may not appear (`{ onError: ... }`). HTML: attribute names such as `onclick` or `style`. JSX attributes such as `<img onError={...}>` are not properties. JSON, YAML and TOML: key paths that may not exist, e.g. `scripts.postinstall` ([key paths](#data-files)). |
| `property_value` | JavaScript and TypeScript: constraints on the value of object-literal properties: `names` (required), then `forbid_literal: true` (the value may not be a string literal) and/or `array_starts_with: accountID` (an array-literal value must start with that element; other values are not checked). `in_call: toast.add` limits it to objects passed directly to that call. JSON, YAML and TOML: `names` are key paths, with `forbid_value` and/or `require_value` (globs, see [Data files](#data-files)). |
| `require_property` | JSON, YAML and TOML. Key paths that must exist, e.g. `jobs.*.timeout-minutes`: every place the parent path (`jobs.*`) matches must contain the key. |
| `same_keys_as` | JSON, YAML and TOML. Path to a reference file. Every file in the rule's scope must have exactly its keys: missing and extra keys are reported. The reference itself is skipped. |
| `same_placeholders` | JSON, YAML and TOML. `true` also requires each shared message to use the same `{{name}}` / `{name}` placeholders as the reference. Needs `same_keys_as`. |
| `literal_argument` | Python, JavaScript, TypeScript and Rust. String literals passed to calls must be listed in a data file, see [Code that must match a data file](#code-that-must-match-a-data-file). |
| `within_property` | JavaScript and TypeScript. Narrows `forbid_call`, `forbid_raise` and `forbid_element` to hits that sit inside an object-literal property with one of these names, e.g. `onError`. |
| `require_call` | Python, JavaScript, TypeScript, Rust, Go and shell. Call patterns, written as in `forbid_call`: at least one must be called in each file in scope, or, with `within_function`, in each function it selects. A file without one is reported on line 1, a function on its `def` line. See [Classes and functions](#classes-and-functions). |
| `require_method` | Python, JavaScript and TypeScript. Method name globs that every selected class must declare in its own body. Inherited methods do not count. |
| `require_field` | Python, JavaScript and TypeScript. Field name globs that every selected class must declare in its own body: annotated or assigned names such as `x: int` or `x = 1` (a TypeScript `x: T` or a class field `x = 1`). |
| `forbid_field` | Python, JavaScript and TypeScript. Field name globs that no selected class may declare. Reported at the field. |
| `in_class` | Python, JavaScript and TypeScript. Which classes `require_method`, `require_field` and `forbid_field` check: `subclass_of: Name` (inheriting from it, directly or not; the class `Name` itself is left out) and/or `names: [globs]`. Without it, every class. |
| `within_function` | Python, JavaScript and TypeScript. Function name globs. `forbid_call` and `forbid_raise` only count inside such a function (nested functions included), and `require_call` is checked per function instead of per file. |
| `except_function` | Python, JavaScript and TypeScript. Function name globs that `within_function` leaves out, e.g. `_*`. Needs `within_function`. |
| `max_lines` | Any language. The most lines a file may have (a positive whole number). A longer file is reported once, on the first line over the limit, with the file's total. `tvlint-ignore-file` skips it. |
| `require_filename` | List of file name globs, matched against the file's base name only (not its directory). Every file in the rule's scope must match at least one, otherwise it is a violation reported at line 1. Applies to all files in scope, not just parseable ones, so `data.json` or `helpers.py` in a `tests/` scope are flagged too. |
| `rationale` | Required. Why the rule exists. Printed with every violation. |
| `fix` | Required. How to comply. Printed with every violation. |

A rule needs at least one check, such as `forbid_raise`, `forbid_call`, `require_call`, `require_method` or `require_filename`. Unknown keys are rejected.

### Top-level keys

| Key | Meaning |
|---|---|
| `rules` | Required. The rules, by id. An id is lowercase words joined by dashes, like `no-direct-db-access`: no slashes, capitals, digits or underscores. |
| `scopes` | Required. Which files each rule runs on. |
| `aliases` | Optional. Import prefixes that stand for a project path: `"@/": "src/"` turns `@/api` into `src/api`. You rarely need it: `forbid_import` rules read `paths`, `baseUrl`, `extends` and `references` from the nearest `tsconfig.json` or `jsconfig.json` (see [Import aliases](#import-aliases)). `aliases` is for what those files cannot say, such as a bundler-only alias, and wins over them. The prefix and the path must both end with `/`, or neither. |

### Scope keys

| Key | Meaning |
|---|---|
| `rules` | Rule ids that run on the matched files. |
| `include` | Glob patterns relative to the project root. `*` stays within one directory, `**` crosses directories. |
| `exclude` | Optional globs to leave out. |

A rule that appears in no scope never runs. Files ignored by `.gitignore` are skipped.

### Nested configs

Any directory below the project root may hold its own `tvlint.yaml`, for a package in a monorepo or a team that owns one corner of the code:

```
tvlint.yaml              # rules for the whole repository
services/billing/
  tvlint.yaml            # rules for services/billing/ only
  error_codes.json
```

`tvl check` finds them on its own; nothing needs to be listed in the root config.

- **Rules add up.** The root config keeps applying everywhere. A nested config adds rules that run on its own directory and below, never outside it. It cannot switch off or change a rule from above.
- **Paths are relative to the config that contains them**: `include` and `exclude` in scopes, `same_keys_as`, `registry.file`, the targets of `aliases`, and project paths in `forbid_import` and `except_import` (a path outside the directory is written `../shared/**`). Violations are still reported relative to the project root.
- **Aliases are inherited.** A nested config sees the aliases of the configs above it, and its own win on the same prefix. A rule uses the aliases of the config it is written in.
- **Rule ids only need to be unique within one config.** Outside the root config a rule is reported with its directory in front: `no-print` in `services/billing/tvlint.yaml` is `services/billing/no-print` in violations, `tvl list` and `tvl explain`. Inside the file, scopes still use the short id.
- A `tvlint.yaml` inside a gitignored directory is not found. `tvl validate` checks every config, each against its own directory, and `tvl explain` says which file a rule comes from.
- The root is the config you point at (`-c`, default `./tvlint.yaml`). Running `tvl` from a subdirectory with no config of its own does not look upward for one.

## License

AGPL-3.0-only. See [LICENSE](LICENSE). To contribute, see [CONTRIBUTING.md](CONTRIBUTING.md); contributors sign a [CLA](CLA.md) so the maintainer can also offer TVLint under other terms.
