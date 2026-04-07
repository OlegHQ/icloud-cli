//! Pattern Matching
//!
//! Converts shell glob patterns to regex equivalents for pattern matching
//! in parameter expansion (${var%pattern}, ${var/pattern/replacement}, etc.)
//! and case statements.
//!
//! ## Error Handling
//!
//! This module follows bash's behavior for invalid patterns:
//! - Invalid character ranges (e.g., `[z-a]`) result in regex compilation failure
//! - Unknown POSIX classes (e.g., `[:foo:]`) produce empty match groups
//! - Unclosed character classes (`[abc`) are treated as literal `[`
//!
//! Callers should wrap regex compilation in try/catch to handle invalid patterns.

use crate::shell::pattern_utils::{
    convert_char_class, find_bracket_end, find_matching_paren, is_regex_special,
    split_extglob_alternatives,
};

/// Convert a shell glob pattern to a regex string.
/// @param pattern - The glob pattern (*, ?, [...])
/// @param greedy - Whether * should be greedy (true for suffix matching, false for prefix)
/// @param extglob - Whether to support extended glob patterns (@(...), *(...), +(...), ?(...), !(...))
pub fn pattern_to_regex(pattern: &str, greedy: bool, extglob: bool) -> String {
    let mut regex = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        // Check for extglob patterns: @(...), *(...), +(...), ?(...), !(...)
        if extglob
            && (c == '@' || c == '*' || c == '+' || c == '?' || c == '!')
            && i + 1 < chars.len()
            && chars[i + 1] == '('
        {
            let close_idx = find_matching_paren(&chars, i + 1);
            if close_idx != usize::MAX {
                let content: String = chars[i + 2..close_idx].iter().collect();
                let alternatives = split_extglob_alternatives(&content);
                let alt_regexes: Vec<String> = alternatives
                    .iter()
                    .map(|alt| pattern_to_regex(alt, greedy, extglob))
                    .collect();
                let alt_group = if !alt_regexes.is_empty() {
                    alt_regexes.join("|")
                } else {
                    "(?:)".to_string()
                };

                match c {
                    '@' => regex.push_str(&format!("(?:{})", alt_group)),
                    '*' => regex.push_str(&format!("(?:{})*", alt_group)),
                    '+' => regex.push_str(&format!("(?:{})+", alt_group)),
                    '?' => regex.push_str(&format!("(?:{})?", alt_group)),
                    '!' => regex.push_str(&format!("(?!(?:{})$).*", alt_group)),
                    _ => {}
                }
                i = close_idx + 1;
                continue;
            }
        }

        if c == '\\' {
            if i + 1 < chars.len() {
                let next = chars[i + 1];
                if is_regex_special(next) {
                    regex.push('\\');
                    regex.push(next);
                } else {
                    regex.push(next);
                }
                i += 2;
            } else {
                regex.push_str("\\\\");
                i += 1;
            }
        } else if c == '*' {
            regex.push_str(if greedy { ".*" } else { ".*?" });
            i += 1;
        } else if c == '?' {
            regex.push('.');
            i += 1;
        } else if c == '[' {
            let class_end = find_bracket_end(&chars, i);
            if class_end == usize::MAX {
                regex.push_str("\\[");
                i += 1;
            } else {
                let class_content: String = chars[i + 1..class_end].iter().collect();
                regex.push_str(&convert_char_class(&class_content));
                i = class_end + 1;
            }
        } else if "^$.|+(){}".contains(c) {
            regex.push('\\');
            regex.push(c);
            i += 1;
        } else {
            regex.push(c);
            i += 1;
        }
    }
    regex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_patterns() {
        assert_eq!(pattern_to_regex("*", true, false), ".*");
        assert_eq!(pattern_to_regex("*", false, false), ".*?");
        assert_eq!(pattern_to_regex("?", true, false), ".");
        assert_eq!(pattern_to_regex("abc", true, false), "abc");
    }

    #[test]
    fn test_escaped_chars() {
        assert_eq!(pattern_to_regex("\\*", true, false), "\\*");
        assert_eq!(pattern_to_regex("\\?", true, false), "\\?");
        assert_eq!(pattern_to_regex("\\[", true, false), "\\[");
    }

    #[test]
    fn test_character_class() {
        assert_eq!(pattern_to_regex("[abc]", true, false), "[abc]");
        assert_eq!(pattern_to_regex("[a-z]", true, false), "[a-z]");
        assert_eq!(pattern_to_regex("[^abc]", true, false), "[^abc]");
    }

    #[test]
    fn test_extglob_patterns() {
        assert_eq!(pattern_to_regex("@(a|b)", true, true), "(?:a|b)");
        assert_eq!(pattern_to_regex("*(a|b)", true, true), "(?:a|b)*");
        assert_eq!(pattern_to_regex("+(a|b)", true, true), "(?:a|b)+");
        assert_eq!(pattern_to_regex("?(a|b)", true, true), "(?:a|b)?");
    }

    #[test]
    fn test_posix_classes() {
        assert_eq!(pattern_to_regex("[[:alpha:]]", true, false), "[a-zA-Z]");
        assert_eq!(pattern_to_regex("[[:digit:]]", true, false), "[0-9]");
    }
}
