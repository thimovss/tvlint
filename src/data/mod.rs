//! JSON, YAML and TOML as one value model: maps, sequences and scalars, each with its position.
//!
//! This is the data counterpart of the language adapters. It is the only place that knows the
//! tree-sitter node kinds of the three formats, so rules can ask questions by path
//! (`jobs.*.runs-on`) without caring which format the file is written in.

mod json;
mod model;
mod path;
mod placeholders;
mod syntax;
mod toml;
mod yaml;

pub use model::{Doc, Position, dotted};
pub use path::PathPattern;
pub use placeholders::placeholders;

#[cfg(test)]
mod tests;
