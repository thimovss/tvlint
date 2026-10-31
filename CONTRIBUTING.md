# Contributing to TVLint

Thanks for helping. This page covers the agreement you sign first, then how to build and check a change.

## Contributor License Agreement

TVLint is open source under the [AGPL-3.0](LICENSE). The maintainer also reserves the right to offer it under other terms, such as a commercial license. That only works if the maintainer may relicense every contribution, so **you must sign the [CLA](CLA.md) before a pull request can be merged**.

- You keep the copyright in your work. The CLA gives the maintainer a license to it.
- You sign once, in your first pull request: a bot comments, and you reply with the sentence it gives you.
- Contributions are always available under the AGPL-3.0 in public releases (section 3 of the CLA).
- If you write code for an employer, make sure you are allowed to contribute it.

## Building and testing

```bash
cargo build
cargo test
cargo clippy --all-targets
cargo run -- check          # tvl checks its own code against tvl.yaml
```

All four must pass. CI runs `tvl validate` and `tvl check` on every pull request.

## What a change should include

- **Tests.** Unit tests sit next to the code; end-to-end CLI tests are in `tests/`.
- **Docs.** A new rule key, flag or command goes in `README.md`. A new rule type or language also goes in `tvl.schema.json` (`tvl schema` prints the current one).
- **Architecture.** `tvl.yaml` encodes the layering of the modules and which crates may be used where. If `tvl check` reports a violation, its `rationale` and `fix` say what to do. Don't suppress it without a reason.
- **Recipes.** Ready-made rules live in `cookbook/`, one `.yaml` file per rule, each with a `rationale` and a `fix`.

## Reporting a problem

Open an issue with the rule file, a small example that shows the problem, and the output you expected.
