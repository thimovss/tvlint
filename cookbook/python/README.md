# Cookbook

Ready-made rules you can copy into your `prlint.yaml`. Each recipe is one file in this directory: a rule plus the scope that applies it. Copy the rule under `rules:` and the scope entry under `scopes:`, adjust the paths to your project, then run:

```bash
prlint validate   # is the config correct, and how many files does each rule run on?
prlint check      # what does it find?
```

All recipes are for Python, the only language supported today.

The cookbook only has rules that depend on your architecture or conventions. Generic problems like `print` calls, `eval`/`exec`, leftover debuggers, unsafe `pickle`/`yaml.load`, `os.system`, blocking `time.sleep` in async code or raising a bare `Exception` are already covered by [Ruff](https://docs.astral.sh/ruff/) (and Bandit, flake8 plugins), so use those instead.

## Before you start

How patterns match, so rules do what you expect:

- **`forbid_raise`** matches the exception's full dotted name or its last part. `HTTPException` also catches `raise fastapi.HTTPException(...)`. A bare `raise` (re-raise) is never flagged.
- **`forbid_call`** matches the callee as it is written. `*` matches anything, including dots. A chained call such as `session.exec(q).one()` is seen as `session.exec().one`, so `*.one` matches it.
- **Imports are not resolved.** `from datetime import datetime` gives `datetime.now`, while `import datetime` gives `datetime.datetime.now`. List both spellings. An alias like `import datetime as dt` is not followed.
- **Paths** in `include` and `exclude` are relative to the directory holding `prlint.yaml`. `*` stays within one directory, `**` crosses directories. Use `exclude` for the one place where the forbidden thing is allowed.
- **Always write `rationale` and `fix`.** They are printed with every violation, and they are what lets a developer or an AI agent correct the code without asking.

Adjust the `src/**` paths in the recipes to your layout. Every file is also a valid config on its own, so you can try one directly:

```bash
prlint validate -c cookbook/python/no-wall-clock-in-tests.yaml
```

## Recipes

### Error handling

- [Keep HTTP out of the domain layer](no-http-in-domain.yaml)
- [Routers must not catch domain exceptions](no-catching-domain-errors-in-routers.yaml)
- [Do not use `.one()` to load a single row](no-query-one.yaml)
- [Every error code raised is listed in error_codes.json](error-codes-are-registered.yaml)

### Determinism

- [No wall clock in tests](no-wall-clock-in-tests.yaml)
- [No unseeded randomness in tests](no-randomness-in-tests.yaml)

### Project layout

- [Test directory files must be named test_*.py or conftest.py](test-pytest-file-names.yaml)

### Architecture

- [Only repositories may commit](commit-only-in-repositories.yaml)
- [No network calls in domain code](no-network-in-domain.yaml)
- [Read configuration in one place](env-only-in-config.yaml)

## What rules cannot express yet

Rules are limited to what the three keys can say: which exceptions are raised, which calls are made, and which exceptions are caught. These are not possible today:

- Import restrictions ("`src/ui` may not import `src/db`").
- Requirements ("every endpoint has an auth decorator", "every service has a test file").
- Rules about the shape of a class, or about arguments passed to a call.
- Following aliases or resolving which module a name really comes from.

For an unsupported convention, write the nearest rule that catches the symptom, and say in `rationale` what the real rule is.
