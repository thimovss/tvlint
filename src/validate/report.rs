//! Collected problems, and how a validation report is printed.

use crate::config::Config;

pub(super) struct Diagnostic {
    pub(super) path: String,
    pub(super) message: String,
    pub(super) help: Option<String>,
}

#[derive(Default)]
pub(super) struct Report {
    pub(super) errors: Vec<Diagnostic>,
    pub(super) warnings: Vec<Diagnostic>,
}

impl Report {
    pub(super) fn error(&mut self, path: &str, message: impl Into<String>, help: Option<String>) {
        self.errors.push(Diagnostic { path: path.into(), message: message.into(), help });
    }

    pub(super) fn warning(&mut self, path: &str, message: impl Into<String>, help: Option<String>) {
        self.warnings.push(Diagnostic { path: path.into(), message: message.into(), help });
    }
}

pub(super) struct RuleSummary {
    pub(super) id: String,
    pub(super) language: String,
    pub(super) forbids: Vec<String>,
    pub(super) files: usize,
    pub(super) scopes: Vec<(usize, usize)>,
}

pub(super) fn print_report(name: &str, report: &Report, config: Option<&Config>, summary: &[RuleSummary]) {
    let warnings = match report.warnings.len() {
        0 => String::new(),
        n => format!(", {n} warning(s)"),
    };
    if report.errors.is_empty() {
        let (rules, scopes) = config.map_or((0, 0), |c| (c.rules.len(), c.scopes.len()));
        println!("{name}: VALID ({rules} rule(s), {scopes} scope(s){warnings})");
    } else {
        println!("{name}: INVALID ({} error(s){warnings})", report.errors.len());
    }
    println!();
    for (label, items) in [("error", &report.errors), ("warning", &report.warnings)] {
        for d in items {
            let at = if d.path.is_empty() { String::new() } else { format!(" at `{}`", d.path) };
            println!("{label}{at}: {}", d.message);
            if let Some(help) = &d.help {
                for (i, line) in help.lines().enumerate() {
                    println!("  {}{line}", if i == 0 { "help: " } else { "      " });
                }
            }
        }
    }
    if !report.errors.is_empty() {
        println!("\nFix the errors above and run `tvl validate` again.");
        return;
    }
    if !report.warnings.is_empty() {
        println!();
    }
    for rule in summary {
        let scopes: Vec<String> = rule.scopes.iter().map(|(i, n)| format!("scopes[{i}]: {n} file(s)")).collect();
        println!("rule {} ({})", rule.id, rule.language);
        println!("  forbids: {}", rule.forbids.join("; "));
        println!("  runs on {} file(s) {}", rule.files, if scopes.is_empty() { "(no scope)".into() } else { format!("[{}]", scopes.join(", ")) });
    }
    println!("\nNext: `tvl check` to find violations, `tvl explain <rule>` to see one rule in full.");
}
