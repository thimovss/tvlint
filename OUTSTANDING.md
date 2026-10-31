# Outstanding questions and action items

Done items are removed from this list.

## Needs a decision

1. **Contributor agreement.** The project is AGPL-3.0-only. To sell a commercial license later, the copyright must be owned or licensed to you: set up a CLA (or copyright assignment) before accepting the first outside contribution, and add a CONTRIBUTING.md.
2. **CLAUDE.md open questions**, still unanswered: shareable rule packs, how much cross-file analysis is worth it.

## Performance

4. **Changed-files-only checks take about 0.65s on a 10k-file project; the target is under 100ms.** A full check of 10k files (mixed Python, TSX, JSON, YAML, 4 rules) now takes about 0.9s on 10 cores (it was 3.0s before parsing, the class index and rule evaluation ran in parallel). A check of one directory is not faster, because prlint still parses every file: class lookups, import resolution and registry files need the whole project. Fix: parse lazily, only the selected files plus what the active rules actually need (Python files only if a rule uses `forbid_catch`, only the referenced registry and reference files). The walk itself is still single-threaded.

## Known limitations

5. **Dotted keys.** Paths are joined with `.`. For `same_keys_as`, `use: path` and "missing key" positions, a key that itself contains a dot (like `a_failed.x`) is ambiguous.
6. **YAML.** `same_keys_as` compares only the first document. Aliases and merge keys (`<<`) are not resolved. List items show as indexes in messages (`containers.1`).
7. **Value checks.** They match text only, so `true` and `"true"` are indistinguishable. Lists and mappings are skipped by `forbid_value` and `require_value`.
8. **Not built:**
   - an expiry-date check for feature flags (depends on the clock, so results would vary)
   - a rule type that runs a command, which would let a rule check README or cookbook sync
9. **Rust and Python.** Calls inside Rust macro arguments are invisible. A Rust rule cannot separate `#[cfg(test)]` code from other code in the same file. Python has no import rules yet.

## Product brief versus reality

CLAUDE.md still describes a product that does not exist yet. Not built:

- `lint check` / `lint init`, and `.lint/rules/`
- JSON and SARIF output
- a mandatory reason on suppressions (optional today)

Decision needed: update the brief to match what exists (`prlint`, `prlint.yaml`), or move these to a roadmap.
