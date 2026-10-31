# Cookbook: TypeScript and JavaScript

Ready-made rules for `prlint.yaml`. Each recipe is one file: a rule plus the scope that applies it. Copy the rule under `rules:` and the scope entry under `scopes:`, adjust the paths, then run `prlint validate` and `prlint check`. See the [Python cookbook](../python/README.md) for how scopes and patterns work in general.

Every recipe uses `language: [javascript, typescript]`, so it covers `.js`, `.jsx`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.mts` and `.cts`. Use `language: typescript` to leave plain JavaScript out.

## How patterns match here

- **`forbid_raise`** matches what is thrown. `throw new HttpException(...)`, `throw HttpException(...)` and `throw errors.HttpException(...)` all name `HttpException`. `throw err` names `err`.
- **`forbid_call`** matches the callee as written. `console.*` matches `console.log(...)`. `new Foo(...)` counts as a call to `Foo`. A chain such as `db.query(sql).one()` is seen as `db.query().one`. Parentheses, `!` and `as` casts are looked through.
- **Imports and aliases are not resolved.** `import { v4 as id } from 'uuid'` makes `id()`, which `uuid.v4` does not match.
- **`forbid_catch` does not apply**, because a JavaScript `catch` clause has no exception type.
- This cookbook only has rules that depend on your architecture or conventions. Generic problems such as `console.log`, `eval` or `debugger` are covered by ESLint.

## Recipes

### Architecture

- [Keep HTTP out of the domain layer](no-http-errors-in-domain.yaml)
- [No network calls in domain code](no-network-in-domain.yaml)

### Tests

- [No wall clock in tests](no-wall-clock-in-tests.yaml)
- [No randomness in tests](no-randomness-in-tests.yaml)
- [Test directory files must be named *.test.ts(x) or *.spec.ts(x)](test-file-names.yaml)
