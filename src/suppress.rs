//! Inline suppression comments, the same in every language:
//!
//! - `tvlint-ignore-next-line` hides every violation reported on the line after the comment.
//! - `tvlint-ignore-file`, as a comment on line 1, excludes the whole file.
//!
//! Directives are read from comment nodes in the syntax tree, so the same words inside a string
//! are ignored. A directive may be followed by a reason: `// tvlint-ignore-next-line: why`.

use std::collections::HashSet;

use crate::check::visit;
use crate::source::SourceFile;

const NEXT_LINE: &str = "tvlint-ignore-next-line";
const FILE: &str = "tvlint-ignore-file";

#[derive(Default)]
pub struct Suppressions {
    /// Line 1 holds a `tvlint-ignore-file` comment.
    pub ignore_file: bool,
    /// 1-based lines whose violations are hidden.
    next_lines: HashSet<usize>,
}

impl Suppressions {
    pub fn scan(file: &SourceFile) -> Suppressions {
        let mut found = Suppressions::default();
        // Most files have no directive; skip walking the tree for them.
        if !file.text.contains("tvlint-ignore") {
            return found;
        }
        let first_line_start = file.text.strip_prefix('\u{feff}').map_or(0, |_| '\u{feff}'.len_utf8());
        visit(file.tree.root_node(), &mut |node| {
            // `comment` in Python, JavaScript and TypeScript; `line_comment` and `block_comment` in Rust.
            if !node.kind().ends_with("comment") {
                return;
            }
            let Some(directive) = directive(&file.text[node.byte_range()]) else { return };
            match directive {
                FILE => {
                    // Line 1, and nothing but whitespace before the comment.
                    let before = &file.text[first_line_start..node.start_byte().max(first_line_start)];
                    if node.start_position().row == 0 && before.trim().is_empty() {
                        found.ignore_file = true;
                    }
                }
                _ => {
                    // The comment may be a block comment spanning lines: the next line is after its end.
                    let mut end_row = node.end_position().row;
                    if file.text[node.byte_range()].ends_with('\n') {
                        end_row -= 1; // some grammars put a line comment's newline inside the node
                    }
                    found.next_lines.insert(end_row + 2);
                }
            }
        });
        found
    }

    pub fn hides(&self, line: usize) -> bool {
        self.next_lines.contains(&line)
    }
}

/// Which directive a comment starts with, if any. The directive must be the first thing in the
/// comment, so prose that merely mentions it does not count.
fn directive(comment: &str) -> Option<&'static str> {
    let body = comment
        .trim()
        .trim_start_matches(['#', '/', '*', '!', '<', '-'])
        .trim_end_matches(['*', '/', '-', '>', ' '])
        .trim_start();
    [NEXT_LINE, FILE].into_iter().find(|name| {
        body.strip_prefix(name)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Parsers;

    fn scan(path: &str, text: &str) -> Suppressions {
        let (language, tree) = Parsers::new().unwrap().parse(path, text).unwrap();
        Suppressions::scan(&SourceFile { path: path.into(), language, text: text.into(), tree })
    }

    fn hidden(path: &str, text: &str) -> Vec<usize> {
        let s = scan(path, text);
        (1..=text.lines().count() + 1).filter(|l| s.hides(*l)).collect()
    }

    #[test]
    fn next_line_comment_forms() {
        assert_eq!(hidden("a.py", "x = 1\n# tvlint-ignore-next-line\ny = 2\nz = 3\n"), [3]);
        assert_eq!(hidden("a.ts", "a;\n// tvlint-ignore-next-line\nb;\nc;\n"), [3]);
        assert_eq!(hidden("a.ts", "a;\n/* tvlint-ignore-next-line */\nb;\nc;\n"), [3]);
        assert_eq!(hidden("a.tsx", "const a = (\n  <div>\n    {/* tvlint-ignore-next-line */}\n    <button />\n  </div>\n);\n"), [4]);
    }

    #[test]
    fn rust_comments() {
        assert_eq!(hidden("a.rs", "fn f() {\n    // tvlint-ignore-next-line\n    x.unwrap();\n    y.unwrap();\n}\n"), [3]);
        assert_eq!(hidden("a.rs", "/* tvlint-ignore-next-line */\nx.unwrap();\n"), [2]);
        assert_eq!(hidden("a.rs", "/// tvlint-ignore-next-line\nfn f() {}\n"), [2]);
        assert!(hidden("a.rs", "let s = \"// tvlint-ignore-next-line\";\nx;\n").is_empty());
        assert!(scan("a.rs", "// tvlint-ignore-file\nfn f() {}\n").ignore_file);
        assert!(!scan("a.rs", "fn f() {}\n// tvlint-ignore-file\n").ignore_file);
    }

    #[test]
    fn shell_comments() {
        assert_eq!(hidden("a.sh", "#!/bin/sh\n# tvlint-ignore-next-line: ok\nrm -rf x\nrm -rf y\n"), [3]);
        assert!(scan("a.sh", "# tvlint-ignore-file\nls\n").ignore_file);
        assert!(hidden("a.sh", "echo \"# tvlint-ignore-next-line\"\nls\n").is_empty());
    }

    #[test]
    fn html_comments() {
        assert_eq!(hidden("a.html", "<!-- tvlint-ignore-next-line: legacy -->\n<font>x</font>\n<font>y</font>\n"), [2]);
        assert_eq!(hidden("a.html", "<p>\n<!--tvlint-ignore-next-line-->\n<font>x</font>\n</p>\n"), [3]);
        assert!(scan("a.html", "<!-- tvlint-ignore-file -->\n<p></p>\n").ignore_file);
    }

    #[test]
    fn a_block_comment_hides_the_line_after_its_end() {
        assert_eq!(hidden("a.ts", "a;\n/* tvlint-ignore-next-line\n   because */\nb;\nc;\n"), [4]);
    }

    #[test]
    fn a_reason_may_follow() {
        assert_eq!(hidden("a.py", "# tvlint-ignore-next-line: legacy endpoint\nx = 1\n"), [2]);
        assert_eq!(hidden("a.ts", "// tvlint-ignore-next-line - generated\nx;\n"), [2]);
    }

    #[test]
    fn only_a_comment_that_starts_with_the_directive_counts() {
        assert!(hidden("a.py", "x = 'tvlint-ignore-next-line'\ny = 2\n").is_empty());
        assert!(hidden("a.ts", "const s = '// tvlint-ignore-next-line';\nx;\n").is_empty());
        assert!(hidden("a.ts", "// we never write tvlint-ignore-next-line here\nx;\n").is_empty());
        assert!(hidden("a.py", "# tvlint-ignore-next-lines\nx = 1\n").is_empty());
        assert!(hidden("a.py", "# tvlint-ignore-next-line-all\nx = 1\n").is_empty());
    }

    #[test]
    fn a_trailing_comment_hides_the_next_line_not_its_own() {
        assert_eq!(hidden("a.py", "x = 1  # tvlint-ignore-next-line\ny = 2\n"), [2]);
    }

    #[test]
    fn ignore_file_must_be_a_comment_on_line_one() {
        assert!(scan("a.py", "# tvlint-ignore-file\nx = 1\n").ignore_file);
        assert!(scan("a.ts", "// tvlint-ignore-file\nx;\n").ignore_file);
        assert!(scan("a.ts", "/* tvlint-ignore-file: generated */\nx;\n").ignore_file);
        assert!(scan("a.ts", "\u{feff}// tvlint-ignore-file\nx;\n").ignore_file);
        assert!(!scan("a.py", "x = 1\n# tvlint-ignore-file\n").ignore_file);
        assert!(!scan("a.py", "\n# tvlint-ignore-file\nx = 1\n").ignore_file);
        assert!(!scan("a.py", "#!/usr/bin/env python\n# tvlint-ignore-file\n").ignore_file);
        assert!(!scan("a.ts", "x; // tvlint-ignore-file\n").ignore_file);
        assert!(!scan("a.ts", "// see tvlint-ignore-file\nx;\n").ignore_file);
        assert!(!scan("a.py", "x = '# tvlint-ignore-file'\n").ignore_file);
    }

    #[test]
    fn ignore_file_does_not_hide_lines() {
        assert!(hidden("a.py", "# tvlint-ignore-file\nx = 1\ny = 2\n").is_empty());
    }
}
