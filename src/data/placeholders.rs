//! The `{{name}}`, `{name}` and ICU placeholders in a message.

use std::collections::BTreeSet;

/// The names between `{{ }}` or `{ }` in a message: `Hello {{name}}, {count, plural, ...}` has
/// `name` and `count`. Two translations of one message must use the same ones.
pub fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '{' {
            i += 1;
            continue;
        }
        let double = chars.get(i + 1) == Some(&'{');
        let start = i + if double { 2 } else { 1 };
        // The group ends at the matching brace; nested groups (plural branches) belong to it.
        let mut depth = 1;
        let mut end = start;
        while end < chars.len() {
            match chars[end] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        let inner: String = chars[start..end.min(chars.len())].iter().collect();
        let name = inner.split(',').next().unwrap_or("").trim();
        if !name.is_empty() && !name.contains(char::is_whitespace) {
            found.insert(name.to_string());
        }
        i = end + 1;
    }
    found
}
