# prlint

Enforce your team's code rules deterministically. Rules live in a YAML file, violations come back with a reason and a fix so both humans and AI agents can act on them.

Supports Python, JavaScript, TypeScript and Rust.

## Install

```bash
cargo install --path .
```

## Usage

Run from the directory that contains `prlint.yaml`:

```bash
prlint
```

```
prlint [OPTIONS] [COMMAND]
```

### Commands

| Command | What it does |
|---|---|
| `check [PATH]...` | Checks the project against all rules and prints every violation. This is the default when no command is given, so `prlint` and `prlint check` are the same. Pass files, directories or glob patterns to check only those (see below). |
| `list` | Lists all configured rules with their language and the number of scopes that use them. |
| `explain <rule>` | Shows why a rule exists, the fix, what it forbids, and which paths it applies to. |
| `validate` | Checks the config file and explains every problem it finds, with the path to the key and how to fix it. Also tells you how many files each rule will run on. Use it after writing or changing a rule. |
| `help [command]` | Prints help for prlint or for one command. |

### Checking specific files

`prlint check` takes any number of files, directories or glob patterns, relative to the directory you run it from. Without any, it checks the whole project.

```bash
prlint check src/api.ts                    # one file
prlint check src/routes src/hooks          # directories: everything below them
prlint check "src/**/*.test.ts"            # a glob: quote it so prlint expands it, not the shell
prlint check -c frontend/prlint.yaml frontend/src/App.tsx   # from the repo root, as a pre-commit hook does
```

- A file is still only checked by the rules whose scope includes it. Passing a path narrows what runs, it never widens a scope.
- Paths must be inside the project, the directory of the config file, and must exist, otherwise prlint exits with `2`. A glob that matches nothing is a warning on stderr. Files ignored by `.gitignore` are skipped, as always.
- Everything is still parsed, so rules that follow imports or class hierarchies see the same project as in a full run, but only the selected files are reported on.
- With `--staged`, a file must be both staged and selected.

### Flags

| Flag | What it does |
|---|---|
| `--staged` | `check` only. Checks only the files staged in git, using their staged contents, so unstaged edits are ignored. Meant for pre-commit hooks. Needs a git repository (exit `2` otherwise). Only staged files under the project root are considered. Combined with `PATH` arguments, a file must be both staged and selected. |
| `-c, --config <path>` | Path to the config file. Defaults to `prlint.yaml` in the current directory. The project root is the directory containing this file, and scope paths are relative to it. Works with every command. |
| `-h, --help` | Prints help. |
| `-V, --version` | Prints the version. |

### Validating rules

`prlint validate` is meant for people and AI agents that add rules. It reports all problems in one pass, not just the first.

- **Errors** make the config invalid (exit `1`): unknown or misspelled keys (with a "did you mean" suggestion), missing `language`, `rationale` or `fix`, wrong types, a rule with no checks, invalid globs, scopes that name a rule that doesn't exist, bad YAML syntax.
- **Warnings** don't fail validation (exit `0`) but usually mean a mistake: a rule no scope uses, a scope that matches no files for the rule's language, a `subclass_of` class that isn't defined in the project. They are only reported once there are no errors.
- When valid, it lists each rule with what it forbids and how many files it runs on. A rule that runs on `0 file(s)` is almost always a wrong glob.

### Exit codes

| Code | Meaning |
|---|---|
| `0` | No violations. |
| `1` | At least one violation was found (`check`), or the config is invalid (`validate`). |
| `2` | prlint could not run: missing or invalid config, unknown rule, bad glob. The message goes to stderr. |

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

Rules are written once and apply to every language they name, for example `language: [javascript, typescript]` for a rule on a mixed codebase. Each language has an adapter that maps its syntax tree to the same few questions the rules ask (what is thrown, what is called, what is imported). Not every language can express every check: Rust has no exceptions, so it has no `forbid_raise` or `forbid_catch`, and only JavaScript and TypeScript have JSX and object-literal checks. `prlint validate` says so when a rule asks a language for something it cannot do.

## Suppressing violations

Two comments switch checks off. They work the same in Python, JavaScript and TypeScript, and are read from real comments, so the same words inside a string do nothing.

| Comment | Effect |
|---|---|
| `prlint-ignore-next-line` | Hides every violation reported on the line directly after the comment. |
| `prlint-ignore-file` | On line 1 of the file, excludes the whole file from every rule, file-name rules included. |

```python
# prlint-ignore-next-line: legacy endpoint, see ISSUE-123
raise ValueError("not found")
```

```tsx
{/* prlint-ignore-next-line */}
<button onClick={open}>Open</button>
```

```ts
// prlint-ignore-file: generated code
```

- Use the comment style of the language: `#` in Python; `//` or `/* */` in JavaScript and TypeScript, and `{/* */}` inside JSX.
- The directive must start the comment. A reason may follow it (`: why`), and is a good idea, but is not required.
- **Next line** means the line right after the comment's last line. A violation is reported on the line where the flagged code starts, so for a statement spanning several lines, put the comment above its first line. A trailing comment (`code  # prlint-ignore-next-line`) hides the next line, not its own.
- **File** needs the comment to be the first thing on line 1. A shebang line or a blank line before it means it does not count. Only Python, JavaScript and TypeScript files can carry it.
- An ignored file is still read to learn its classes, so other files' `forbid_catch` results do not change when a file opts out.
- Nothing is silent: the summary line says how many violations were hidden and how many files were skipped, e.g. `prlint: 7 violation(s) in 4 file(s) [5 hidden by prlint-ignore-next-line, 3 file(s) skipped by prlint-ignore-file]`.

## Config

Looking for rules to start from? See the [cookbook](cookbook/).

`prlint.yaml` has two sections: `rules` says what is forbidden, `scopes` says which files each rule runs on.

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
| `language` | `python`, `javascript`, `typescript` or `rust`, or a list such as `[javascript, typescript]`. Limits the rule to files of those languages. Required with `forbid_*` checks. Optional with `require_filename`, where omitting it applies the rule to every file type. |
| `forbid_raise` | Python, JavaScript and TypeScript. Exception names that may not be raised (`throw` in JavaScript and TypeScript, where `throw new Foo()` names `Foo`). Matches the full dotted name or its last part, so `HTTPException` also matches `fastapi.HTTPException`. Bare `raise` (re-raise) is allowed. |
| `forbid_call` | Call patterns that are forbidden. `*` matches anything, so `*.one` matches `session.exec(q).one()` and `console.*` matches `console.log(...)`. In JavaScript and TypeScript, `new Foo()` counts as a call to `Foo`. In Rust, functions are written with `::` (`std::fs::read`, `Vec::new`), methods with `.` (`*.unwrap` matches `s.parse::<i32>().unwrap()`), and macros with their `!` (`println!`); calls inside a macro's arguments are not seen. Imports and aliases are not resolved. |
| `forbid_catch` | Python only, because JavaScript and TypeScript `catch` clauses have no exception type. `subclass_of: Name` forbids `except` clauses that catch `Name` or any class inheriting from it. Subclasses are found by class name across the whole project, not by import path. |
| `forbid_import` | JavaScript, TypeScript and Rust. Module specifiers that may not be imported (`import`, `export ... from`, `import()`, `require()`; in Rust `use` and `extern crate`, where `use a::{b, c}` counts as `a::b` and `a::c`, and `self::` and `super::` are resolved to `crate::` paths). Relative imports and imports through an alias are matched by their project path (`src/hooks/use-mobile.ts`), anything else by the specifier as written (`@base-ui/react/dialog`). Patterns are path globs: `*` stays within one segment, `**` crosses segments, so list a package and its subpaths (`react-aria-components`, `react-aria-components/**`). Rust patterns are written with `::` (`sqlx::**`, `crate::db::**`). |
| `except_import` | Imports that are fine although they match `forbid_import`. Together they express "this folder may only import from here": `forbid_import: ["src/**"]` with `except_import: ["src/components/ui/**"]`. |
| `forbid_element` | JavaScript and TypeScript. JSX element names that may not be used, e.g. `[button, textarea]`. Matched on the tag as written, so `button` does not match `Button`. |
| `forbid_property` | JavaScript and TypeScript. Object-literal property names that may not appear (`{ onError: ... }`). JSX attributes such as `<img onError={...}>` are not properties. |
| `property_value` | JavaScript and TypeScript. Constraints on the value of object-literal properties: `names` (required), then `forbid_literal: true` (the value may not be a string literal) and/or `array_starts_with: accountID` (an array-literal value must start with that element; other values are not checked). `in_call: toast.add` limits it to objects passed directly to that call. |
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
