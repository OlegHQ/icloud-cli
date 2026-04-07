//! Glob Helper Functions for GLOBIGNORE and Pattern-to-Regex
//!
//! This module provides helper functions for glob pattern handling that are
//! used by the GlobExpander. These functions are distinct from the pattern
//! matching in `interpreter::expansion::pattern` which is for parameter
//! expansion contexts.
//!
//! ## Functions
//!
//! - `split_globignore_patterns` — Split GLOBIGNORE env var on colons
//! - `globignore_pattern_to_regex` — Convert GLOBIGNORE pattern to regex (star does not match `/`)
//! - `glob_to_regex` — Convert glob pattern to regex for filename matching

use super::pattern_utils::{
    convert_char_class, expand_posix_classes_in_regex, find_bracket_end, is_regex_special,
};

/// Split the GLOBIGNORE environment variable value on colons, preserving
/// colons that appear inside `[...]` character classes (including POSIX
/// classes like `[[:alnum:]]`) and escaped colons (`\:`).
///
/// # Examples
/// ```
/// use just_bash::shell::glob_helpers::split_globignore_patterns;
/// assert_eq!(split_globignore_patterns("*.txt:*.log"), vec!["*.txt", "*.log"]);
/// ```
pub fn split_globignore_patterns(globignore: &str) -> Vec<String> {
    if globignore.is_empty() {
        return Vec::new();
    }

    let mut patterns = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = globignore.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c == '\\' && i + 1 < chars.len() {
            // Escaped character – always literal (including \:)
            current.push(c);
            current.push(chars[i + 1]);
            i += 2;
            continue;
        }

        if c == '[' {
            // Find matching ] and copy the whole bracket expression
            let end = find_bracket_end(&chars, i);
            if end != usize::MAX {
                let bracket: String = chars[i..=end].iter().collect();
                current.push_str(&bracket);
                i = end + 1;
            } else {
                current.push(c);
                i += 1;
            }
            continue;
        }

        if c == ':' {
            patterns.push(current);
            current = String::new();
            i += 1;
            continue;
        }

        current.push(c);
        i += 1;
    }

    patterns.push(current);
    patterns
}

/// Convert a GLOBIGNORE pattern to a regex string where `*` does NOT match `/`.
///
/// This is used to test whether a filename should be excluded from glob results.
/// In GLOBIGNORE context, `*` matches any sequence of characters except `/`,
/// and `?` matches any single character except `/`.
///
/// The result is anchored with `^...$`.
pub fn globignore_pattern_to_regex(pattern: &str) -> String {
    let mut regex = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if is_regex_special(next) {
                regex.push('\\');
            }
            regex.push(next);
            i += 2;
        } else if c == '*' {
            regex.push_str("[^/]*");
            i += 1;
        } else if c == '?' {
            regex.push_str("[^/]");
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
        } else if is_regex_special(c) {
            regex.push('\\');
            regex.push(c);
            i += 1;
        } else {
            regex.push(c);
            i += 1;
        }
    }

    regex.push('$');
    regex
}

/// Convert a glob pattern to a regex string for filename matching.
///
/// Uses `brush_parser::pattern::pattern_to_regex_str()` for the core conversion,
/// then expands POSIX character classes for `regex_lite` compatibility.
///
/// Unlike `globignore_pattern_to_regex`, here `*` matches anything (including `/`)
/// because this is used for matching individual filenames against patterns
/// (used by GlobExpander.matchPattern).
///
/// When `extglob` is true, extended glob patterns are supported:
/// - `@(pat1|pat2)` — match exactly one of the patterns
/// - `*(pat1|pat2)` — match zero or more occurrences
/// - `+(pat1|pat2)` — match one or more occurrences
/// - `?(pat1|pat2)` — match zero or one occurrence
/// - `!(pat1|pat2)` — match anything except the patterns
///
/// The result is anchored with `^...$`.
pub fn glob_to_regex(pattern: &str, extglob: bool) -> String {
    match brush_parser::pattern::pattern_to_regex_str(pattern, extglob) {
        Ok(inner) => {
            let expanded = expand_posix_classes_in_regex(&inner);
            format!("^{}$", expanded)
        }
        Err(_) => {
            // Fallback: treat the whole pattern as a literal
            let escaped: String = pattern.chars().map(|c| {
                if is_regex_special(c) { format!("\\{}", c) } else { c.to_string() }
            }).collect();
            format!("^{}$", escaped)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use regex_lite::Regex;

    // -----------------------------------------------------------------------
    // split_globignore_patterns tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_split_simple_colon_separated() {
        assert_eq!(
            split_globignore_patterns("*.txt:*.log"),
            vec!["*.txt", "*.log"]
        );
    }

    #[test]
    fn test_split_posix_class_preserved() {
        assert_eq!(
            split_globignore_patterns("[[:alnum:]]*:*.txt"),
            vec!["[[:alnum:]]*", "*.txt"]
        );
    }

    #[test]
    fn test_split_escaped_colon() {
        assert_eq!(split_globignore_patterns("a\\:b:c"), vec!["a\\:b", "c"]);
    }

    #[test]
    fn test_split_empty_string() {
        let result: Vec<String> = split_globignore_patterns("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_split_single_pattern() {
        assert_eq!(split_globignore_patterns("single"), vec!["single"]);
    }

    #[test]
    fn test_split_multiple_posix_classes() {
        assert_eq!(
            split_globignore_patterns("[[:digit:]][[:alpha:]]:*.rs"),
            vec!["[[:digit:]][[:alpha:]]", "*.rs"]
        );
    }

    #[test]
    fn test_split_negated_bracket() {
        assert_eq!(
            split_globignore_patterns("[!abc]*:*.log"),
            vec!["[!abc]*", "*.log"]
        );
    }

    #[test]
    fn test_split_caret_negated_bracket() {
        assert_eq!(
            split_globignore_patterns("[^abc]*:*.log"),
            vec!["[^abc]*", "*.log"]
        );
    }

    // -----------------------------------------------------------------------
    // globignore_pattern_to_regex tests
    // -----------------------------------------------------------------------

    fn gi_matches(pattern: &str, text: &str) -> bool {
        let regex_str = globignore_pattern_to_regex(pattern);
        let re = Regex::new(&regex_str).unwrap();
        re.is_match(text)
    }

    #[test]
    fn test_gi_star_matches_filename() {
        assert!(gi_matches("*", "foo"));
    }

    #[test]
    fn test_gi_star_does_not_match_path() {
        assert!(!gi_matches("*", "dir/foo"));
    }

    #[test]
    fn test_gi_star_txt_matches_file() {
        assert!(gi_matches("*.txt", "file.txt"));
    }

    #[test]
    fn test_gi_star_txt_does_not_match_path() {
        assert!(!gi_matches("*.txt", "dir/file.txt"));
    }

    #[test]
    fn test_gi_question_matches_single_char() {
        assert!(gi_matches("?", "a"));
    }

    #[test]
    fn test_gi_question_does_not_match_slash() {
        assert!(!gi_matches("?", "/"));
    }

    #[test]
    fn test_gi_char_class_matches() {
        assert!(gi_matches("[abc]*", "afile"));
    }

    #[test]
    fn test_gi_char_class_does_not_match() {
        assert!(!gi_matches("[abc]*", "dfile"));
    }

    // -----------------------------------------------------------------------
    // glob_to_regex tests
    // -----------------------------------------------------------------------

    fn glob_matches(pattern: &str, text: &str, extglob: bool) -> bool {
        let regex_str = glob_to_regex(pattern, extglob);
        let re = Regex::new(&regex_str).unwrap();
        re.is_match(text)
    }

    #[test]
    fn test_glob_star_matches_anything() {
        assert!(glob_matches("*", "anything", false));
    }

    #[test]
    fn test_glob_star_txt_matches() {
        assert!(glob_matches("*.txt", "file.txt", false));
    }

    #[test]
    fn test_glob_star_txt_does_not_match_rs() {
        assert!(!glob_matches("*.txt", "file.rs", false));
    }

    #[test]
    fn test_glob_question_matches_single() {
        assert!(glob_matches("?", "a", false));
    }

    #[test]
    fn test_glob_question_does_not_match_two() {
        assert!(!glob_matches("?", "ab", false));
    }

    #[test]
    fn test_glob_char_class_matches() {
        assert!(glob_matches("[abc]", "a", false));
    }

    #[test]
    fn test_glob_char_class_does_not_match() {
        assert!(!glob_matches("[abc]", "d", false));
    }

    #[test]
    fn test_glob_negated_class_matches() {
        assert!(glob_matches("[!abc]", "d", false));
    }

    #[test]
    fn test_glob_negated_class_does_not_match() {
        assert!(!glob_matches("[!abc]", "a", false));
    }

    #[test]
    fn test_glob_posix_digit_matches() {
        assert!(glob_matches("[[:digit:]]", "5", false));
    }

    #[test]
    fn test_glob_posix_digit_does_not_match_alpha() {
        assert!(!glob_matches("[[:digit:]]", "a", false));
    }

    #[test]
    fn test_glob_extglob_at_matches() {
        assert!(glob_matches("@(foo|bar)", "foo", true));
        assert!(glob_matches("@(foo|bar)", "bar", true));
        assert!(!glob_matches("@(foo|bar)", "baz", true));
    }

    #[test]
    fn test_glob_extglob_star_matches() {
        assert!(glob_matches("*(ab)", "", true));
        assert!(glob_matches("*(ab)", "ab", true));
        assert!(glob_matches("*(ab)", "abab", true));
        assert!(!glob_matches("*(ab)", "abc", true));
    }

    #[test]
    fn test_glob_extglob_plus_matches() {
        assert!(!glob_matches("+(ab)", "", true));
        assert!(glob_matches("+(ab)", "ab", true));
        assert!(glob_matches("+(ab)", "abab", true));
    }

    #[test]
    fn test_glob_extglob_question_matches() {
        assert!(glob_matches("?(ab)", "", true));
        assert!(glob_matches("?(ab)", "ab", true));
        assert!(!glob_matches("?(ab)", "abab", true));
    }

    #[test]
    fn test_glob_escaped_star_matches_literal() {
        assert!(glob_matches("\\*", "*", false));
        assert!(!glob_matches("\\*", "foo", false));
    }

    #[test]
    fn test_glob_escaped_question_matches_literal() {
        assert!(glob_matches("\\?", "?", false));
        assert!(!glob_matches("\\?", "a", false));
    }

    #[test]
    fn test_glob_posix_alpha_class() {
        assert!(glob_matches("[[:alpha:]]", "a", false));
        assert!(glob_matches("[[:alpha:]]", "Z", false));
        assert!(!glob_matches("[[:alpha:]]", "5", false));
    }

    #[test]
    fn test_glob_posix_alnum_class() {
        assert!(glob_matches("[[:alnum:]]", "a", false));
        assert!(glob_matches("[[:alnum:]]", "5", false));
        assert!(!glob_matches("[[:alnum:]]", "!", false));
    }

    #[test]
    fn test_glob_dot_is_literal() {
        assert!(glob_matches("file.txt", "file.txt", false));
        assert!(!glob_matches("file.txt", "fileatxt", false));
    }

    #[test]
    fn test_glob_caret_negated_class() {
        assert!(glob_matches("[^abc]", "d", false));
        assert!(!glob_matches("[^abc]", "a", false));
    }

    #[test]
    fn test_glob_range_class() {
        assert!(glob_matches("[a-z]", "m", false));
        assert!(!glob_matches("[a-z]", "M", false));
    }

    #[test]
    fn test_glob_extglob_not_enabled() {
        assert!(!glob_matches("@(foo|bar)", "foo", false));
    }

    #[test]
    fn test_glob_star_matches_empty() {
        assert!(glob_matches("*", "", false));
    }

    #[test]
    fn test_glob_complex_pattern() {
        assert!(glob_matches("*.tar.gz", "archive.tar.gz", false));
        assert!(!glob_matches("*.tar.gz", "archive.tar.bz2", false));
    }

    #[test]
    fn test_glob_unclosed_bracket_is_literal() {
        assert!(glob_matches("\\[abc", "[abc", false));
    }
}
