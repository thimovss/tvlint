//! `tvl validate`: checks the config file for mistakes and explains every problem.

mod code_keys;
mod data_keys;
mod keys;
mod project;
mod report;
mod rule;
mod structure;
mod util;

use std::path::Path;

use anyhow::{Context, Result};
use serde_yaml_ng::Value;

use crate::config::Config;
use crate::source;

use project::check_against_project;
use report::{Report, print_report};
use structure::validate_structure;

/// Validates the config file and every nested one below it, each against its own directory. Returns whether all are valid.
pub fn run_all(config_path: &Path) -> Result<bool> {
    let mut valid = run(config_path, &config_path.display().to_string())?;
    let own = std::path::absolute(config_path)?;
    let root = own.parent().context("config file has no parent directory")?;
    for file in source::find_configs(root, &own)? {
        println!();
        valid &= run(&root.join(&file), &file)?;
    }
    // Rules from different configs share one namespace, which only loading them together can check.
    if valid && let Err(err) = source::load_config(config_path) {
        println!("\nerror: {err:#}");
        valid = false;
    }
    Ok(valid)
}

pub(super) fn run(config_path: &Path, name: &str) -> Result<bool> {
    let text = std::fs::read_to_string(config_path).with_context(|| format!("cannot read {name}"))?;

    let value: Value = match serde_yaml_ng::from_str(&text) {
        Ok(value) => value,
        Err(err) => {
            println!("{name}: INVALID (1 error)\n");
            let at = err.location().map_or(String::new(), |l| format!(" (line {}, column {})", l.line(), l.column()));
            println!("error: not valid YAML{at}");
            println!("  {err}");
            return Ok(false);
        }
    };

    let mut report = Report::default();
    let referenced = validate_structure(&value, &mut report);

    // The typed loader is what `check` uses, so it has the final say.
    let config = if report.errors.is_empty() {
        match Config::parse(&text) {
            Ok(config) => Some(config),
            Err(err) => {
                report.error("", format!("{err:#}"), None);
                None
            }
        }
    } else {
        None
    };

    let mut summary = Vec::new();
    if let Some(config) = &config {
        summary = check_against_project(config, config_path, &referenced, &mut report)?;
    }

    print_report(name, &report, config.as_ref(), &summary);
    Ok(report.errors.is_empty())
}

#[cfg(test)]
mod tests;
