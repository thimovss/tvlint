# prlint

Enforce your team's code rules deterministically. Rules live in a YAML file, violations come back with a reason and a fix so both humans and AI agents can act on them.

Currently supports Python.

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
| `check` | Checks the project against all rules and prints every violation. This is the default when no command is given, so `prlint` and `prlint check` are the same. |
| `list` | Lists all configured rules with their language and the number of scopes that use them. |
| `explain <rule>` | Shows why a rule exists, the fix, what it forbids, and which paths it applies to. |
| `validate` | Checks the config file and explains every problem it finds, with the path to the key and how to fix it. Also tells you how many files each rule will run on. Use it after writing or changing a rule. |
| `help [command]` | Prints help for prlint or for one command. |

### Flags

| Flag | What it does |
|---|---|
| `--staged` | `check` only. Checks only the files staged in git, using their staged contents, so unstaged edits are ignored. Meant for pre-commit hooks. Needs a git repository (exit `2` otherwise). Only staged files under the project root are considered. |
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

## Config

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
| `language` | Required. Selects the parser. Only `python` for now. |
| `forbid_raise` | Exception names that may not be raised. Matches the full dotted name or its last part, so `HTTPException` also matches `fastapi.HTTPException`. Bare `raise` (re-raise) is allowed. |
| `forbid_call` | Call patterns that are forbidden. `*` matches anything, so `*.one` matches `session.exec(q).one()`. |
| `forbid_catch` | `subclass_of: Name` forbids `except` clauses that catch `Name` or any class inheriting from it. Subclasses are found by class name across the whole project, not by import path. |
| `rationale` | Required. Why the rule exists. Printed with every violation. |
| `fix` | Required. How to comply. Printed with every violation. |

A rule needs at least one of `forbid_raise`, `forbid_call` or `forbid_catch`. Unknown keys are rejected.

### Scope keys

| Key | Meaning |
|---|---|
| `rules` | Rule ids that run on the matched files. |
| `include` | Glob patterns relative to the project root. `*` stays within one directory, `**` crosses directories. |
| `exclude` | Optional globs to leave out. |

A rule that appears in no scope never runs. Files ignored by `.gitignore` are skipped.
