# Cookbook: TypeScript and JavaScript

Ready-made rules for `tvlint.yaml`. Each recipe is one file: a rule plus the scope that applies it. Copy the rule under `rules:` and the scope entry under `scopes:`, adjust the paths, then run `tvl validate` and `tvl check`. See the [Python cookbook](../python/README.md) for how scopes and patterns work in general.

Every recipe uses `language: [javascript, typescript]`, so it covers `.js`, `.jsx`, `.mjs`, `.cjs`, `.ts`, `.tsx`, `.mts` and `.cts`. Use `language: typescript` to leave plain JavaScript out.

## How patterns match here

- **`forbid_raise`** matches what is thrown. `throw new HttpException(...)`, `throw HttpException(...)` and `throw errors.HttpException(...)` all name `HttpException`. `throw err` names `err`.
- **`forbid_call`** matches the callee as written. `console.*` matches `console.log(...)`. `new Foo(...)` counts as a call to `Foo`. A chain such as `db.query(sql).one()` is seen as `db.query().one`. Parentheses, `!` and `as` casts are looked through.
- **Imports and aliases are not resolved.** `import { v4 as id } from 'uuid'` makes `id()`, which `uuid.v4` does not match.
- **`forbid_import`** matches the project path of a relative or aliased import (`src/hooks/use-mobile.ts`) and the specifier as written for a package (`@base-ui/react/dialog`). Patterns are path globs: `*` stays within one segment, `**` crosses segments. Add `aliases: { "@/": "src/" }` so `@/...` imports are seen as project paths.
- **`forbid_element`** matches JSX tags as written: `button` is the HTML element, `Button` is not.
- **`property_value` and `forbid_property`** look at object literals only. JSX attributes such as `<img onError={...}>` are not properties.
- **`forbid_catch` does not apply**, because a JavaScript `catch` clause has no exception type.
- This cookbook only has rules that depend on your architecture or conventions. Generic problems such as `console.log`, `eval` or `debugger` are covered by ESLint.

## Recipes

### Architecture

- [Keep HTTP out of the domain layer](no-http-errors-in-domain.yaml)
- [No network calls in domain code](no-network-in-domain.yaml)

### Imports and layering

- [Use the design-system wrappers, not the primitive library](design-primitives-only-in-ui.yaml)
- [The design system must not import application code](ui-is-self-contained.yaml)
- [Shared components must not import feature code](shared-components-no-feature-imports.yaml)

### Components and data

- [Use the design system's components, not raw HTML elements](use-design-system-elements.yaml)
- [Do not toast API failures a second time](no-toast-in-on-error.yaml)
- [Toast text must come from the translation function](toasts-are-translated.yaml)
- [Query keys start with the tenant or account id](query-keys-scoped-by-account.yaml)
- [Every t("key") exists in the English translations](translation-keys-exist.yaml)

### Tests

- [No wall clock in tests](no-wall-clock-in-tests.yaml)
- [No randomness in tests](no-randomness-in-tests.yaml)
- [Test directory files must be named *.test.ts(x) or *.spec.ts(x)](test-file-names.yaml)
