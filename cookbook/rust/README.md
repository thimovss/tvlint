# Cookbook: Rust

Ready-made rules for `tvl.yaml`. Each recipe is one file: a rule plus the scope that applies it. Copy the rule under `rules:` and the scope entry under `scopes:`, adjust the paths and module names, then run `tvl validate` and `tvl check`. See the [Python cookbook](../python/README.md) for how scopes work in general.

This cookbook only has rules that depend on your architecture. Generic problems such as `unwrap`, `println!`, `dbg!` and `todo!` are covered by Clippy's restriction lints.

## How patterns match in Rust

- **`forbid_call`** matches the callee as written. Functions use `::` (`std::fs::read_to_string`, `Vec::new`), methods use `.` (`x.unwrap`), and `*` matches anything: `*.unwrap` matches `opt.unwrap()` and `s.parse::<i32>().unwrap()`. A macro is written with its `!`: `println!`, `std::println!`. Generics, `?` and `.await` are looked through.
- **Calls inside a macro's arguments are not seen.** Tree-sitter keeps them as flat tokens, so `assert_eq!(x.unwrap(), 1)` is one call, to `assert_eq!`.
- **`forbid_import`** matches `use` and `extern crate`. `use a::{b, c::d}` is checked as `a::b` and `a::c::d`. Write patterns with `::`, and list a crate and its paths separately (`sqlx` and `sqlx::**`). `*` stays within one segment, `**` crosses segments.
- **`self::` and `super::`** are resolved to `crate::` paths from the file's place under `src/` (`src/api/handlers.rs` is `crate::api::handlers`), including inside an inline `mod tests { ... }`. A pattern written with `crate::` therefore matches both spellings.
- **Not available:** `forbid_raise` and `forbid_catch`. Rust has no exceptions, so `validate` rejects them.
- A `#[cfg(test)] mod tests` lives in the same file as the code it tests, so a scope cannot separate test code from the rest. Scope test rules to the `tests/` directory.

## Recipes

### Architecture

- [The domain layer must not depend on infrastructure](domain-is-pure.yaml)
- [One place talks HTTP](one-http-client.yaml)

### Tests

- [No wall clock in integration tests](no-wall-clock-in-tests.yaml)
- [No unseeded randomness in integration tests](no-randomness-in-tests.yaml)
