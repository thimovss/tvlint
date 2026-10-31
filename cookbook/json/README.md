# Cookbook: JSON

Ready-made rules for `prlint.yaml`. Each recipe is one file: a rule plus the scope that applies it. Copy the rule under `rules:` and the scope entry under `scopes:`, adjust the paths to your layout, then run `prlint validate` and `prlint check`. See the [Python cookbook](../python/README.md) for how scopes work in general.

Rules on JSON, YAML and TOML read **key paths** (`dependencies.*`, `compilerOptions.strict`), not syntax. See [Data files](../../README.md#data-files) for the path syntax; the same rules work in all three formats. Values in `forbid_value` and `require_value` are globs: write a literal `*` as `[*]`.

## Recipes

### Dependencies

- [Pin dependencies to exact versions](pinned-npm-versions.yaml)
- [No install-time scripts](no-install-scripts.yaml)
- [Link internal packages with the workspace protocol](workspace-packages-use-workspace-protocol.yaml)

### Configuration

- [TypeScript runs in strict mode](tsconfig-is-strict.yaml)

### Translations

- [Every translation file has the same keys and placeholders](locale-files-match.yaml)

See also [translation keys that exist](../typescript/translation-keys-exist.yaml) and [error codes that are registered](../python/error-codes-are-registered.yaml), which tie code to a JSON file.
