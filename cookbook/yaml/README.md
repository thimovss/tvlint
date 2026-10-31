# Cookbook: YAML

Ready-made rules for `prlint.yaml`. Each recipe is one file: a rule plus the scope that applies it. Copy the rule under `rules:` and the scope entry under `scopes:`, adjust the paths to your layout, then run `prlint validate` and `prlint check`. See the [Python cookbook](../python/README.md) for how scopes work in general.

Rules on JSON, YAML and TOML read **key paths** (`jobs.*.timeout-minutes`), not syntax. In YAML, list items are addressed by index, so `*` also steps into lists, and every document of a multi-document file is checked. See [Data files](../../README.md#data-files). Values in `forbid_value` and `require_value` are globs: write a literal `*` as `[*]`.

## Recipes

### GitHub Actions

- [Pin actions to a commit SHA](actions-pinned-to-sha.yaml)
- [Every job has a timeout](workflow-jobs-have-timeouts.yaml)
- [Workflows declare their permissions](workflows-declare-permissions.yaml)

### Containers

- [Container images have a version tag](no-latest-image-tags.yaml)
- [Kubernetes containers declare limits and probes](kubernetes-containers-have-limits.yaml)

### Configuration

- [Every feature flag has an owner](feature-flags-have-owners.yaml)
