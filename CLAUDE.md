# Codebase Linter: Product Brief

*Working name. Business-rule enforcement for AI-assisted codebases.*

## Problem

AI coding tools write code that is locally correct but globally inconsistent. They invent new helpers instead of reusing existing ones, bypass architectural layers, and ignore conventions that were never written down. Reviewers catch this by hand, which doesn't scale as AI-generated volume grows. Existing linters cover syntax and style, not "how we do things here."

## Product

A fast, Rust-based CLI that enforces team-defined business and architecture rules, written in plain YAML and checked deterministically. Violations come back in a form both humans and AI agents can act on, so agents can self-correct before a human ever sees the PR.

**Positioning:** not a general-purpose linter. It is the enforcement layer for the rules your team has, but nobody has encoded.

**Multi-language by design:** the product must work across languages, not just one ecosystem. The same rule file should apply to a polyglot repo (e.g. a TypeScript frontend and a Python backend) and produce consistent results. We start with **JavaScript, TypeScript, and Python**, but nothing in the core engine, rule schema, or output formats may be hard-wired to a specific language. Adding a language should mean adding a tree-sitter grammar and a language adapter, not changing the engine or the rules.

**Data files are first-class:** JSON, YAML, and TOML are read into one path-addressable value model (`jobs.*.timeout-minutes`), so the same rule works in every format. Rules can also tie code to data, e.g. every error code raised must be registered in `error_codes.json`, or every translation file must have the same keys.

## Target Users

- **Primary:** engineering teams (5-100 devs) adopting AI coding tools who see growing review burden and drift.
- **Buyer/champion:** tech leads and platform teams who own architecture standards.

## MVP Scope

**In scope**

- Single binary CLI (`lint check`, `lint init`, `lint explain <rule>`)
- YAML rule files in `.lint/rules/`
- Parsing via tree-sitter; **JavaScript, TypeScript, and Python** first, behind a language-agnostic adapter layer (see Architecture)
- Four rule types:
  1. **Dependency boundaries:** `src/ui` may not import from `src/db`
  2. **Banned patterns:** forbid specific calls, imports, or syntax
  3. **Required patterns:** endpoints must carry an auth decorator
  4. **File relationships:** `*.service.ts` requires a sibling `*.service.test.ts`
- Output formats: human-readable, JSON, SARIF (for GitHub code scanning)
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
- **Language adapters:** each supported language (JS, TS, Python at launch) implements a common adapter interface that maps its tree-sitter AST into the shared normalized model (imports, symbols, decorators/annotations, calls). Rule evaluation only sees the normalized model, so rules are language-independent. Rules may optionally scope to languages (e.g. `languages: [typescript, python]`); by default they apply to every language where the rule type makes sense. Language-specific concerns (module resolution, `tsconfig` paths, Python package roots, file extensions) live in the adapter, never in the engine.
- **Rule loader:** `serde_yaml` with strict schema validation and clear error messages.

**Performance targets:** under 1s for a 10k-file repo; under 100ms for changed-files-only checks.

## Differentiators

| Existing tools | Gap |
|---|---|
| ESLint, Ruff | Style and bug patterns, not business rules |
| Semgrep, ast-grep | Powerful but pattern-authoring heavy, not agent-aware |
| ArchUnit, dependency-cruiser | Single-ecosystem architecture checks |
| AI review tools | Non-deterministic, unsuitable as merge gates |

Ours: simple declarative rules, deterministic results, and agent-readable feedback.

## Roadmap

1. **MVP (8-10 weeks):** engine, four rule types, CLI, JS/TS + Python via the language adapter layer.
2. **v0.2:** rule-hit analytics.
3. **v0.3:** rule mining from the existing codebase ("92% of endpoints use X, 3 don't"), proposed as draft YAML.
4. **v1.0:** optional LLM-evaluated rules (advisory by default, promotable to blocking), hosted dashboard, more languages.

## Risks

- **False-positive fatigue** kills linters. Mitigate with precise rule semantics, and easy but audited suppression.
- **Rule authoring friction.** Mitigate with good defaults, templates, and rule mining.
- **Overlap with Semgrep.** Mitigate by staying focused on business rules and agent feedback, not breadth of pattern matching.
- **Parser maintenance across languages.** Mitigate by starting with JS/TS and Python, relying on tree-sitter's ecosystem, and keeping language-specific logic isolated in adapters so new languages are cheap to add.
- **Language leakage into the core.** Easy to let one language's quirks (e.g. TS path aliases) shape the engine or rule schema. Mitigate by testing every rule type against more than one language.

## Success Metrics

- Time to first useful rule: under 10 minutes
- Rule false-positive rate: under 5%
- Weekly active repos and rules enforced per repo
- Share of violations fixed by agents without human intervention

## Decisions

- **Open source under AGPL-3.0-only.** Open source is non-negotiable. The network clause covers hosted use, and a commercial license (dual licensing) is the way to make money later. Because that needs copyright ownership, outside contributions need a CLA or copyright assignment before they are accepted.

## Open Questions

- Should rules be shareable as packs (e.g. "clean architecture", "NestJS conventions")?
- How much cross-file analysis (call graphs, type info) is worth the complexity versus staying import- and syntax-level?
