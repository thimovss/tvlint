//! Key paths such as `jobs.*.runs-on`, and matching them against the keys of a document.

use anyhow::{Result, bail};
use globset::{Glob, GlobMatcher};

pub(super) enum Segment {
    Literal(String),
    Glob(GlobMatcher),
    /// `*`: any one key or array index.
    Any,
    /// `**`: any number of keys, including none.
    Deep,
}

/// A dotted path with wildcards, anchored at the root of the file: `scripts.postinstall`,
/// `jobs.*.runs-on`, `**.version`. Array items are addressed by index, so `*` also steps into lists.
/// A key that contains a dot is written in double quotes: `"a.b".c`.
pub struct PathPattern {
    pub(super) text: String,
    pub(super) segments: Vec<Segment>,
}

impl PathPattern {
    pub fn parse(text: &str) -> Result<PathPattern> {
        let mut segments = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut was_quoted = false;
        let mut finish = |current: &mut String, was_quoted: &mut bool| -> Result<()> {
            if current.is_empty() {
                bail!("`{text}` has an empty segment");
            }
            let segment = if *was_quoted {
                Segment::Literal(std::mem::take(current))
            } else {
                match current.as_str() {
                    "*" => Segment::Any,
                    "**" => Segment::Deep,
                    s if s.contains(['*', '?', '[']) => Segment::Glob(Glob::new(s)?.compile_matcher()),
                    _ => Segment::Literal(current.clone()),
                }
            };
            current.clear();
            *was_quoted = false;
            segments.push(segment);
            Ok(())
        };
        for c in text.chars() {
            match c {
                '"' => {
                    quoted = !quoted;
                    was_quoted = true;
                }
                '.' if !quoted => finish(&mut current, &mut was_quoted)?,
                c => current.push(c),
            }
        }
        if quoted {
            bail!("`{text}` has an unclosed quote");
        }
        finish(&mut current, &mut was_quoted)?;
        Ok(PathPattern { text: text.to_string(), segments })
    }

    pub fn matches(&self, path: &[String]) -> bool {
        matches_from(&self.segments, path)
    }

    /// Whether the pattern names exactly one place, with no wildcard.
    pub fn is_literal(&self) -> bool {
        self.segments.iter().all(|s| matches!(s, Segment::Literal(_)))
    }

    /// The pattern for the container, and the key that must be in it: `jobs.*.runs-on` is
    /// (`jobs.*`, `runs-on`). `None` if the last segment is a wildcard, which names no single key.
    pub fn split_last(&self) -> Option<(PathPattern, String)> {
        let (last, parent) = self.segments.split_last()?;
        let Segment::Literal(key) = last else { return None };
        let text = self.text.rsplit_once('.').map_or(String::new(), |(parent, _)| parent.to_string());
        let parent = PathPattern {
            text,
            segments: parent
                .iter()
                .map(|s| match s {
                    Segment::Literal(l) => Segment::Literal(l.clone()),
                    Segment::Glob(g) => Segment::Glob(g.glob().compile_matcher()),
                    Segment::Any => Segment::Any,
                    Segment::Deep => Segment::Deep,
                })
                .collect(),
        };
        Some((parent, key.clone()))
    }
}

pub(super) fn matches_from(segments: &[Segment], path: &[String]) -> bool {
    let Some((first, rest)) = segments.split_first() else { return path.is_empty() };
    match first {
        Segment::Deep => (0..=path.len()).any(|skip| matches_from(rest, &path[skip..])),
        _ => {
            let Some((key, remaining)) = path.split_first() else { return false };
            let hit = match first {
                Segment::Literal(l) => l == key,
                Segment::Glob(g) => g.is_match(key),
                _ => true,
            };
            hit && matches_from(rest, remaining)
        }
    }
}
