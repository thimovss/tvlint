# Cookbook: TOML

Ready-made rules for `prlint.yaml`. Each recipe is one file: a rule plus the scope that applies it. Copy the rule under `rules:` and the scope entry under `scopes:`, adjust the paths to your layout, then run `prlint validate` and `prlint check`. See the [Python cookbook](../python/README.md) for how scopes work in general.

Rules on JSON, YAML and TOML read **key paths** (`package.license`, `dependencies.*`), not syntax. Tables (`[a.b]`), dotted keys, inline tables and arrays of tables (`[[bin]]`, addressed by index) all end up as the same paths. See [Data files](../../README.md#data-files). Values in `forbid_value` and `require_value` are globs: write a literal `*` as `[*]`.

## Recipes

### Rust

- [No `"*"` dependency requirements](no-wildcard-dependency-versions.yaml)
- [Every crate declares its license and Rust version](crates-declare-metadata.yaml)

### Python

- [pyproject.toml declares the Python version](pyproject-pins-python.yaml)
