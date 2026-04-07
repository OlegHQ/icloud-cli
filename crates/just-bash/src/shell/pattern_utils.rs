//! Shared Pattern Utilities
//!
//! Common functions for glob-to-regex conversion used by both:
//! - `shell::glob_helpers` (filename pattern matching)
//! - `interpreter::expansion::pattern` (parameter expansion patterns)
//! - `commands::awk` and `commands::sed` (POSIX class expansion)

/// POSIX character class names mapped to regex-lite-compatible character ranges.
/// Used across glob matching, parameter expansion, awk, and sed.
pub fn posix_class_to_regex(name: &str) -> &'static str {
    match name {
        "alnum" => "a-zA-Z0-9",
        "alpha" => "a-zA-Z",
        "ascii" => "\\x00-\\x7F",
        "blank" => " \\t",
        "cntrl" => "\\x00-\\x1F\\x7F",
        "digit" => "0-9",
        "graph" => "!-~",
        "lower" => "a-z",
        "print" => " -~",
        "punct" => "!-/:-@\\[-`{-~",
        "space" => " \\t\\n\\r\\f\\v",
        "upper" => "A-Z",
        "word" => "a-zA-Z0-9_",
        "xdigit" => "0-9A-Fa-f",
        _ => "",
    }
}

/// Check if a character is a regex special character that needs escaping.
pub fn is_regex_special(c: char) -> bool {
    "\\^$.|+(){}[]*?".contains(c)
}

/// Find the matching closing parenthesis for an open paren at `open_idx`,
/// handling nesting and escape sequences. Returns `usize::MAX` if not found.
pub fn find_matching_paren(chars: &[char], open_idx: usize) -> usize {
    let mut depth = 1;
    let mut i = open_idx + 1;
    while i < chars.len() && depth > 0 {
        let c = chars[i];
        if c == '\\' {
            i += 2; // Skip escaped char
            continue;
        }
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    usize::MAX
}

/// Split extglob pattern content on `|`, handling nested parentheses
/// and escape sequences.
pub fn split_extglob_alternatives(content: &str) -> Vec<String> {
    let mut alternatives: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            current.push(c);
            if i + 1 < chars.len() {
                current.push(chars[i + 1]);
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if c == '(' {
            depth += 1;
            current.push(c);
        } else if c == ')' {
            depth -= 1;
            current.push(c);
        } else if c == '|' && depth == 0 {
            alternatives.push(current);
            current = String::new();
        } else {
            current.push(c);
        }
        i += 1;
    }
    alternatives.push(current);
    alternatives
}

/// Find the end of a bracket expression `[...]` starting at `start` (where
/// `chars[start]` is `[`). Returns the index of the closing `]`, or
/// `usize::MAX` if no matching `]` is found.
///
/// Handles: `!`/`^` negation, literal `]` after `[`, POSIX classes `[:name:]`,
/// escape sequences, and single-quoted literals (bash extension).
pub fn find_bracket_end(chars: &[char], start: usize) -> usize {
    let mut i = start + 1;

    // Handle negation prefix
    if i < chars.len() && (chars[i] == '!' || chars[i] == '^') {
        i += 1;
    }

    // A ] immediately after [ or [! or [^ is literal, not closing
    if i < chars.len() && chars[i] == ']' {
        i += 1;
    }

    while i < chars.len() {
        // Handle escape sequences - \] should not end the class
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 2;
            continue;
        }

        if chars[i] == ']' {
            return i;
        }

        // Handle single quotes inside character class (bash extension)
        if chars[i] == '\'' {
            let rest: String = chars[i + 1..].iter().collect();
            if let Some(close_quote) = rest.find('\'') {
                i = i + 1 + close_quote + 1;
                continue;
            }
        }

        // Handle POSIX classes [:name:]
        if chars[i] == '[' && i + 1 < chars.len() && chars[i + 1] == ':' {
            let rest: String = chars[i + 2..].iter().collect();
            if let Some(close_pos) = rest.find(":]") {
                i = i + 2 + close_pos + 2;
                continue;
            }
        }

        i += 1;
    }

    usize::MAX
}

/// Convert the content inside a shell character class `[...]` to a regex
/// character class. The input is the content between `[` and `]` (exclusive).
///
/// Handles `!`/`^` negation, POSIX classes `[:name:]`, escape sequences,
/// single-quoted literals (bash extension), and ranges.
pub fn convert_char_class(content: &str) -> String {
    let mut result = String::from("[");
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    // Handle negation: bash uses ! but regex uses ^
    if !chars.is_empty() && (chars[0] == '^' || chars[0] == '!') {
        result.push('^');
        i += 1;
    }

    while i < chars.len() {
        // Handle single quotes inside character class (bash extension)
        if chars[i] == '\'' {
            let rest: String = chars[i + 1..].iter().collect();
            if let Some(close_quote) = rest.find('\'') {
                let quoted: String = chars[i + 1..i + 1 + close_quote].iter().collect();
                for ch in quoted.chars() {
                    if ch == '\\' {
                        result.push_str("\\\\");
                    } else if ch == ']' {
                        result.push_str("\\]");
                    } else if ch == '^' && result == "[" {
                        result.push_str("\\^");
                    } else {
                        result.push(ch);
                    }
                }
                i = i + 1 + close_quote + 1;
                continue;
            }
        }

        // Handle POSIX classes like [:alpha:]
        if chars[i] == '[' && i + 1 < chars.len() && chars[i + 1] == ':' {
            let rest: String = chars[i + 2..].iter().collect();
            if let Some(close_pos) = rest.find(":]") {
                let class_name: String = chars[i + 2..i + 2 + close_pos].iter().collect();
                result.push_str(posix_class_to_regex(&class_name));
                i = i + 2 + close_pos + 2;
                continue;
            }
        }

        // Handle escape sequences
        if chars[i] == '\\' && i + 1 < chars.len() {
            result.push('\\');
            result.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // Regular character
        result.push(chars[i]);
        i += 1;
    }

    result.push(']');
    result
}

/// Expand POSIX character classes in a regex pattern string using simple
/// string replacement. Used by awk where patterns come as pre-built strings.
pub fn expand_posix_classes_in_pattern(pattern: &str) -> String {
    // Build replacements from the shared class definitions
    let classes = [
        "space", "blank", "alpha", "digit", "alnum", "upper", "lower",
        "punct", "xdigit", "graph", "print", "cntrl",
    ];
    let mut result = pattern.to_string();
    for name in &classes {
        let from = format!("[[:{name}:]]");
        let to = format!("[{}]", posix_class_to_regex(name));
        result = result.replace(&from, &to);
    }
    result
}
