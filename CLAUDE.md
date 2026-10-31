# Codebase Linter: Product Brief

*Working name. Business-rule enforcement for AI-assisted codebases.*

## Problem

AI coding tools write code that is locally correct but globally inconsistent. They invent new helpers instead of reusing existing ones, bypass architectural layers, and ignore conventions that were never written down. Reviewers catch this by hand, which doesn't scale as AI-generated volume grows. Existing linters cover syntax and style, not "how we do things here."

## Product

A fast, Rust-based CLI that enforces team-defined business and architecture rules, written in plain YAML and checked deterministically. Violations come back in a form both humans and AI agents can act on, so agents can self-correct before a human ever sees the PR.

**Positioning:** not a general-purpose linter. It is the enforcement layer for the rules your team has, but nobody has encoded.

## Target Users

- **Primary:** engineering teams (5-100 devs) adopting AI coding tools who see growing review burden and drift.
- **Buyer/champion:** tech leads and platform teams who own architecture standards.

## MVP Scope

**In scope**

- Single binary CLI (`lint check`, `lint init`, `lint explain <rule>`)
- YAML rule files in `.lint/rules/`
- Parsing via tree-sitter; **TypeScript and Python** first
- Four rule types:
  1. **Dependency boundaries:** `src/ui` may not import from `src/db`
  2. **Banned patterns:** forbid specific calls, imports, or syntax
  3. **Required patterns:** endpoints must carry an auth decorator
  4. **File relationships:** `*.service.ts` requires a sibling `*.service.test.ts`
- Output formats: human-readable, JSON, SARIF (for GitHub code scanning)
- GitHub Action for PR checks
- Inline suppressions with a mandatory reason

**Out of scope for MVP:** LLM-evaluated rules, rule mining, hosted dashboard, IDE plugin, additional languages.

## Example Rule

```yaml
id: no-direct-db-access
description: Database access must go through the repository layer.
severity: error
type: dependency
from: "src/**"
except_from: "src/repositories/**"
forbid_import: ["src/db/**", "pg", "knex"]
rationale: Direct queries bypass auditing and caching.
fix: Use the matching repository in src/repositories/.
```

Every rule carries a `rationale` and a `fix` field. These are returned in the output so an AI agent knows why the rule exists and how to comply.

## Architecture

- **Core engine (Rust):** file discovery (`ignore` crate), parallel parsing (`rayon` + tree-sitter), rule evaluation against a normalized import/symbol graph.
- **Rule loader:** `serde_yaml` with strict schema validation and clear error messages.
- **Incremental mode:** cache parse results by file hash; check only changed files in PRs.
- **Agent interface:** MCP server mode (`lint serve`) so coding agents can query rules before writing code and validate output after.

**Performance targets:** under 1s for a 10k-file repo on a warm cache; under 100ms for changed-files-only checks.

## Differentiators

| Existing tools | Gap |
|---|---|
| ESLint, Ruff | Style and bug patterns, not business rules |
| Semgrep, ast-grep | Powerful but pattern-authoring heavy, not agent-aware |
| ArchUnit, dependency-cruiser | Single-ecosystem architecture checks |
| AI review tools | Non-deterministic, unsuitable as merge gates |

Ours: simple declarative rules, deterministic results, agent-readable feedback, and an MCP interface.

## Roadmap

1. **MVP (8-10 weeks):** engine, four rule types, CLI, GitHub Action, TS + Python.
2. **v0.2:** MCP server, `lint init` that scaffolds starter rules, rule-hit analytics.
3. **v0.3:** rule mining from the existing codebase ("92% of endpoints use X, 3 don't"), proposed as draft YAML.
4. **v1.0:** optional LLM-evaluated rules (advisory by default, promotable to blocking), hosted dashboard, more languages.

## Risks

- **False-positive fatigue** kills linters. Mitigate with precise rule semantics, easy but audited suppression, and a dry-run mode for new rules.
- **Rule authoring friction.** Mitigate with good defaults, templates, and rule mining.
- **Overlap with Semgrep.** Mitigate by staying focused on business rules and agent feedback, not breadth of pattern matching.
- **Parser maintenance across languages.** Mitigate by starting with two and relying on tree-sitter's ecosystem.

## Success Metrics

- Time to first useful rule: under 10 minutes
- Rule false-positive rate: under 5%
- Weekly active repos and rules enforced per repo
- Share of violations fixed by agents without human intervention

## Open Questions

- Open-source core with a paid hosted layer, or fully commercial?
- Should rules be shareable as packs (e.g. "clean architecture", "NestJS conventions")?
- How much cross-file analysis (call graphs, type info) is worth the complexity versus staying import- and syntax-level?
