//! Placeholders in import patterns: `{1}` is the text the first wildcard segment of the scope's
//! `include` pattern matched, `{2}` the second, and so on. They let one scope say "a package may
//! import itself" without listing every package.

use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobMatcher};

/// An `include` pattern that remembers what its wildcard segments matched.
pub struct CapturePattern {
    segments: Vec<Segment>,
}

enum Segment {
    /// `**`: any number of path segments, none of them captured.
    Any,
    Literal(String),
    /// A segment with a wildcard in it (`*`, `?`, `[..]`, `{..}`). The whole path segment is captured.
    Wildcard(GlobMatcher),
}

impl CapturePattern {
    pub fn new(pattern: &str) -> Result<CapturePattern> {
        let segments = pattern
            .trim_start_matches("./")
            .split('/')
            .map(|segment| {
                Ok(if segment == "**" {
                    Segment::Any
                } else if segment.contains(['*', '?', '[', '{']) {
                    let glob = GlobBuilder::new(segment).literal_separator(true).build().with_context(|| format!("invalid glob `{pattern}`"))?;
                    Segment::Wildcard(glob.compile_matcher())
                } else {
                    Segment::Literal(segment.to_string())
                })
            })
            .collect::<Result<_>>()?;
        Ok(CapturePattern { segments })
    }

    /// How many placeholders this pattern can fill: its segments with a wildcard in them.
    pub fn wildcards(&self) -> usize {
        self.segments.iter().filter(|s| matches!(s, Segment::Wildcard(_))).count()
    }

    /// What each wildcard segment matched in `path`, in order; `None` if the pattern does not match the path.
    pub fn captures(&self, path: &str) -> Option<Vec<String>> {
        let parts: Vec<&str> = path.split('/').collect();
        let mut found = Vec::new();
        self.matches(0, &parts, &mut found).then_some(found)
    }

    fn matches(&self, at: usize, parts: &[&str], found: &mut Vec<String>) -> bool {
        let Some(segment) = self.segments.get(at) else { return parts.is_empty() };
        match segment {
            Segment::Any => (0..=parts.len()).any(|skip| {
                let mark = found.len();
                let ok = self.matches(at + 1, &parts[skip..], found);
                if !ok {
                    found.truncate(mark);
                }
                ok
            }),
            Segment::Literal(text) => parts.first() == Some(&text.as_str()) && self.matches(at + 1, &parts[1..], found),
            Segment::Wildcard(glob) => {
                let Some(part) = parts.first().filter(|p| glob.is_match(p)) else { return false };
                found.push((*part).to_string());
                let ok = self.matches(at + 1, &parts[1..], found);
                if !ok {
                    found.pop();
                }
                ok
            }
        }
    }
}

/// The numbers of the placeholders (`{1}`, `{2}`, ...) written in `pattern`.
pub fn placeholders(pattern: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else { break };
        if let Ok(n) = rest[..close].parse::<usize>()
            && n >= 1
        {
            found.push(n);
        }
        rest = &rest[close + 1..];
    }
    found
}

/// `pattern` with each `{n}` replaced by the n-th capture. A placeholder without a capture is left as written.
pub fn fill(pattern: &str, captures: &[String]) -> String {
    let mut out = pattern.to_string();
    for n in placeholders(pattern) {
        if let Some(text) = captures.get(n - 1) {
            out = out.replace(&format!("{{{n}}}"), &escape(text));
        }
    }
    out
}

/// `text` as a glob that matches exactly itself: a directory called `[id]` must not read as a character class.
fn escape(text: &str) -> String {
    text.chars().map(|c| if "*?[]{}\\".contains(c) { format!("[{c}]") } else { c.to_string() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(pattern: &str, path: &str) -> Option<Vec<String>> {
        CapturePattern::new(pattern).unwrap().captures(path)
    }

    #[test]
    fn wildcard_segments_are_captured_whole_and_in_order() {
        assert_eq!(caps("src/*/commands/**", "src/assets/commands/rename.py"), Some(vec!["assets".to_string()]));
        assert_eq!(caps("src/*/*.py", "src/assets/aggregate.py"), Some(vec!["assets".to_string(), "aggregate.py".to_string()]));
    }

    #[test]
    fn double_star_matches_any_depth_and_captures_nothing() {
        assert_eq!(caps("src/**/events/*.py", "src/a/b/events/x.py"), Some(vec!["x.py".to_string()]));
        assert_eq!(caps("src/**/events/*.py", "src/events/x.py"), Some(vec!["x.py".to_string()]));
        assert_eq!(caps("**", "a/b"), Some(vec![]));
    }

    #[test]
    fn a_pattern_that_does_not_match_has_no_captures() {
        assert_eq!(caps("src/*/commands/**", "src/assets/events/x.py"), None);
        assert_eq!(caps("src/*", "src/a/b"), None);
    }

    #[test]
    fn placeholders_are_found_and_filled() {
        assert_eq!(placeholders("src/{1}/**/{2}.py"), [1, 2]);
        assert_eq!(placeholders("src/{a,b}/**"), Vec::<usize>::new());
        assert_eq!(fill("src/{1}/**", &["assets".to_string()]), "src/assets/**");
        assert_eq!(fill("src/{2}/**", &["assets".to_string()]), "src/{2}/**");
        assert_eq!(fill("src/{1}/**", &["[id]".to_string()]), "src/[[]id[]]/**");
    }

    #[test]
    fn counts_the_placeholders_a_pattern_can_fill() {
        assert_eq!(CapturePattern::new("src/*/commands/**/*.py").unwrap().wildcards(), 2);
        assert_eq!(CapturePattern::new("src/**").unwrap().wildcards(), 0);
    }
}
