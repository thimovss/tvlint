# Outstanding questions and action items

Status after commit `e3ae059` (JSON, YAML and TOML support).

## Needs a decision

1. **Branch.** The repo is on `master`; the main branch is `main`. Commits are local, nothing is pushed. PR, or merge into `main`?
2. **`schema/prlint.schema.json`.** Untracked, and not created in this work. It looks like a JSON schema for `prlint.yaml`. If it is yours, it is missing the new languages (`json`, `yaml`, `toml`) and keys (`require_property`, `same_keys_as`, `same_placeholders`, `literal_argument`, `forbid_value`, `require_value`). Update it? Commit it?
3. **`.idea/linter.iml`.** One-line change not made in this work. Commit, ignore or revert?
4. **Hidden directories.** prlint now reads dot-directories such as `.github/` (needed for workflow rules). This changes behaviour for every project. Acceptable, or should it be an option?
5. **CLAUDE.md open questions**, still unanswered: open-core vs commercial, shareable rule packs, how much cross-file analysis is worth it.

## Not verified

6. **`example/` projects.** `example/prlint.yaml` (5 violations expected) and `example/frontend/prlint.yaml` (47 expected) have not been re-run since data files and the hidden-directory change.
7. **Cookbook recipes.** Only some of the new ones were run end to end: `no-latest-image-tags`, `kubernetes-containers-have-limits`, `tsconfig-is-strict`, `translation-keys-exist`, the error-codes recipe. The others were only checked for valid structure: `actions-pinned-to-sha`, `workflows-declare-permissions`, `pinned-npm-versions`, `workspace-packages-use-workspace-protocol`, `locale-files-match`, `feature-flags-have-owners`, and the three TOML recipes.
8. **Performance.** Every JSON, YAML and TOML file is now parsed, lockfiles included. Not measured against the targets in CLAUDE.md (under 1s for 10k files; under 100ms for changed-files-only).
9. **Missing tests:**
   - hidden-directory walk
   - the "registry file not found" error in `check`
   - unused-entry reporting with `--staged` or `PATH` arguments
   - YAML and TOML suppression end to end
   - project-level `validate` checks (missing or empty registry)
   - TOML edge cases (quoted keys in headers, duplicate keys)

## Known limitations

10. **Dotted keys.** Paths are joined with `.`. For `same_keys_as`, `use: path` and "missing key" positions, a key that itself contains a dot (like `a_failed.x`) is ambiguous.
11. **YAML.** `same_keys_as` compares only the first document. Aliases and merge keys (`<<`) are not resolved. List items show as indexes in messages (`containers.1`).
12. **Value checks.** They match text only, so `true` and `"true"` are indistinguishable. Lists and mappings are skipped by `forbid_value` and `require_value`.
13. **README accuracy.** It says JSON suppression needs `.jsonc` files. The JSON grammar also accepts comments in plain `.json`; test it and correct the text.
14. **Not built:**
    - an expiry-date check for feature flags (depends on the clock, so results would vary)
    - a rule type that runs a command, which would let a rule check README or cookbook sync
15. **Rust and Python.** Calls inside Rust macro arguments are invisible. A Rust rule cannot separate `#[cfg(test)]` code from other code in the same file. Python has no import rules yet.

## Product brief versus reality

CLAUDE.md still describes a product that does not exist yet. Not built:

- `lint check` / `lint init` / `lint serve`, and `.lint/rules/`
- JSON and SARIF output
- the GitHub Action
- the MCP server
- the incremental parse cache and `rayon` parallelism
- a dry-run mode for new rules
- a mandatory reason on suppressions (optional today)

Decision needed: update the brief to match what exists (`prlint`, `prlint.yaml`), or move these to a roadmap.

## Suggested order

1. Items 6 and 7: quick, and they confirm nothing regressed.
2. Items 9 and 13: tests and a doc fix.
3. Items 1 to 4: decisions that unblock pushing.
4. Item 8: measure before optimising.
