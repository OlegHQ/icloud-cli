use crate::shell::pattern_utils::posix_class_to_regex;

/// Map POSIX character class names to their character ranges.
fn posix_class(name: &str) -> Option<&'static str> {
    let result = posix_class_to_regex(name);
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// Convert Basic Regular Expression (BRE) to Extended Regular Expression (ERE).
///
/// In BRE: `+`, `?`, `|`, `(`, `)` are literal; `\+`, `\?`, `\|`, `\(`, `\)` are special
/// In ERE: those chars are special without backslash
pub fn bre_to_ere(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    let mut in_bracket = false;

    while i < chars.len() {
        if chars[i] == '[' && !in_bracket {
            // Standalone POSIX class [[:space:]]
            if i + 2 < chars.len() && chars[i + 1] == '[' && chars[i + 2] == ':' {
                if let Some(close_idx) = find_posix_close(&chars, i + 3) {
                    let class_name: String = chars[i + 3..close_idx].iter().collect();
                    if let Some(js_class) = posix_class(&class_name) {
                        result.push('[');
                        result.push_str(js_class);
                        result.push(']');
                        i = close_idx + 3;
                        continue;
                    }
                }
            }

            // Negated standalone POSIX class [^[:space:]]
            if i + 3 < chars.len()
                && chars[i + 1] == '^'
                && chars[i + 2] == '['
                && chars[i + 3] == ':'
            {
                if let Some(close_idx) = find_posix_close(&chars, i + 4) {
                    let class_name: String = chars[i + 4..close_idx].iter().collect();
                    if let Some(js_class) = posix_class(&class_name) {
                        result.push_str("[^");
                        result.push_str(js_class);
                        result.push(']');
                        i = close_idx + 3;
                        continue;
                    }
                }
            }

            result.push('[');
            i += 1;
            in_bracket = true;

            if i < chars.len() && chars[i] == '^' {
                result.push('^');
                i += 1;
            }
            if i < chars.len() && chars[i] == ']' {
                result.push_str("\\]");
                i += 1;
            }
            continue;
        }

        if in_bracket {
            if chars[i] == ']' {
                result.push(']');
                i += 1;
                in_bracket = false;
                continue;
            }

            if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == ':' {
                if let Some(close_idx) = find_posix_close_inside(&chars, i + 2) {
                    let class_name: String = chars[i + 2..close_idx].iter().collect();
                    if let Some(js_class) = posix_class(&class_name) {
                        result.push_str(js_class);
                        i = close_idx + 2;
                        continue;
                    }
                }
            }

            if chars[i] == '\\' && i + 1 < chars.len() {
                result.push(chars[i]);
                result.push(chars[i + 1]);
                i += 2;
                continue;
            }

            result.push(chars[i]);
            i += 1;
            continue;
        }

        // Outside brackets — BRE to ERE conversion
        if chars[i] == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if matches!(next, '+' | '?' | '|' | '(' | ')' | '{' | '}') {
                result.push(next);
                i += 2;
                continue;
            }
            if next == 't' {
                result.push('\t');
                i += 2;
                continue;
            }
            if next == 'n' {
                result.push('\n');
                i += 2;
                continue;
            }
            if next == 'r' {
                result.push('\r');
                i += 2;
                continue;
            }
            result.push(chars[i]);
            result.push(next);
            i += 2;
            continue;
        }

        if matches!(chars[i], '+' | '?' | '|' | '(' | ')') {
            result.push('\\');
            result.push(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '^' {
            let is_anchor = result.is_empty() || result.ends_with('(');
            if !is_anchor {
                result.push_str("\\^");
                i += 1;
                continue;
            }
        }

        if chars[i] == '$' {
            let is_end = i == chars.len() - 1;
            let before_group_close =
                i + 2 < chars.len() && chars[i + 1] == '\\' && chars[i + 2] == ')';
            if !is_end && !before_group_close {
                result.push_str("\\$");
                i += 1;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

fn find_posix_close(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 2 < chars.len() {
        if chars[i] == ':' && chars[i + 1] == ']' && chars[i + 2] == ']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_posix_close_inside(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == ':' && chars[i + 1] == ']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Preprocess a sed script to convert BRE regex patterns to ERE.
///
/// Identifies regex patterns in address positions and `s` commands,
/// applies `bre_to_ere()` to those portions only. Leaves replacement
/// strings, text arguments, filenames, and labels untouched.
pub fn preprocess_bre_script(script: &str) -> String {
    let chars: Vec<char> = script.chars().collect();
    let len = chars.len();
    let mut result = String::new();
    let mut i = 0;

    while i < len {
        // Whitespace / semicolons / braces / negation — pass through
        if chars[i].is_whitespace() || matches!(chars[i], ';' | '{' | '}' | '!' | ',') {
            result.push(chars[i]);
            i += 1;
            continue;
        }

        // Line-number or $ address — copy digits/$ and continue
        if chars[i].is_ascii_digit() || chars[i] == '$' {
            while i < len
                && (chars[i].is_ascii_digit() || chars[i] == '~' || chars[i] == '$')
            {
                result.push(chars[i]);
                i += 1;
            }
            continue;
        }

        // Address pattern /regex/
        if chars[i] == '/' {
            result.push('/');
            i += 1;
            let (pattern, new_i) = extract_delimited(&chars, i, '/');
            result.push_str(&bre_to_ere(&pattern));
            result.push('/');
            i = new_i;
            continue;
        }

        // Address pattern \cregexc (alternate delimiter)
        if chars[i] == '\\' && i + 1 < len && !chars[i + 1].is_alphanumeric() {
            let delim = chars[i + 1];
            result.push('\\');
            result.push(delim);
            i += 2;
            let (pattern, new_i) = extract_delimited(&chars, i, delim);
            result.push_str(&bre_to_ere(&pattern));
            result.push(delim);
            i = new_i;
            continue;
        }

        // s command — convert only the pattern (first section)
        if chars[i] == 's' && i + 1 < len && !chars[i + 1].is_alphanumeric() {
            result.push('s');
            i += 1;
            let delim = chars[i];
            result.push(delim);
            i += 1;
            // Pattern section — convert BRE→ERE
            let (pattern, new_i) = extract_delimited(&chars, i, delim);
            result.push_str(&bre_to_ere(&pattern));
            result.push(delim);
            i = new_i;
            // Replacement section — copy verbatim
            while i < len && chars[i] != delim {
                if chars[i] == '\\' && i + 1 < len {
                    result.push(chars[i]);
                    result.push(chars[i + 1]);
                    i += 2;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            if i < len {
                result.push(delim);
                i += 1;
            }
            // Flags — copy verbatim until command boundary
            while i < len && !matches!(chars[i], ';' | '\n' | '}') {
                result.push(chars[i]);
                i += 1;
            }
            continue;
        }

        // y command — three delimited sections, no regex, copy verbatim
        if chars[i] == 'y' && i + 1 < len && !chars[i + 1].is_alphanumeric() {
            result.push('y');
            i += 1;
            let delim = chars[i];
            let mut delim_count = 0;
            while i < len && delim_count < 3 {
                if chars[i] == '\\' && i + 1 < len {
                    result.push(chars[i]);
                    result.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if chars[i] == delim {
                    delim_count += 1;
                }
                result.push(chars[i]);
                i += 1;
            }
            continue;
        }

        // Commands with text/filename/label arguments (no regex) —
        // a, i, c, r, w, R, W, b, t, T, : — copy to command boundary
        if "aicrwRWbtT:".contains(chars[i]) {
            while i < len && !matches!(chars[i], ';' | '\n' | '}') {
                result.push(chars[i]);
                i += 1;
            }
            continue;
        }

        // Single-char commands (d, D, p, P, h, H, g, G, x, n, N, q, Q, =, l, z, F, #)
        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Extract content between delimiters, handling escapes and bracket expressions.
/// Returns (content, position after closing delimiter).
fn extract_delimited(chars: &[char], start: usize, delim: char) -> (String, usize) {
    let mut content = String::new();
    let mut i = start;

    while i < chars.len() && chars[i] != delim {
        if chars[i] == '\\' && i + 1 < chars.len() {
            content.push(chars[i]);
            content.push(chars[i + 1]);
            i += 2;
        } else if chars[i] == '[' {
            content.push('[');
            i += 1;
            if i < chars.len() && chars[i] == '^' {
                content.push('^');
                i += 1;
            }
            if i < chars.len() && chars[i] == ']' {
                content.push(']');
                i += 1;
            }
            while i < chars.len() && chars[i] != ']' {
                content.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                content.push(']');
                i += 1;
            }
        } else {
            content.push(chars[i]);
            i += 1;
        }
    }

    if i < chars.len() {
        i += 1; // skip closing delimiter
    }

    (content, i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bre_to_ere_basic() {
        assert_eq!(bre_to_ere(r"\+"), "+");
        assert_eq!(bre_to_ere(r"\(foo\)"), "(foo)");
    }

    #[test]
    fn test_bre_literal_plus() {
        assert_eq!(bre_to_ere("+"), r"\+");
    }

    #[test]
    fn test_posix_class_expansion() {
        assert_eq!(bre_to_ere("[[:alpha:]]"), "[a-zA-Z]");
        assert_eq!(bre_to_ere("[[:digit:]]"), "[0-9]");
    }

    #[test]
    fn test_negated_posix_class() {
        assert_eq!(bre_to_ere("[^[:digit:]]"), "[^0-9]");
    }

    #[test]
    fn test_posix_class_inside_bracket() {
        let result = bre_to_ere("[a[:space:]b]");
        assert!(result.contains(" \\t"));
    }

    #[test]
    fn test_bre_escape_sequences() {
        assert_eq!(bre_to_ere(r"\t"), "\t");
        assert_eq!(bre_to_ere(r"\n"), "\n");
    }

    #[test]
    fn test_bracket_expression_passthrough() {
        assert_eq!(bre_to_ere("[abc]"), "[abc]");
        assert_eq!(bre_to_ere("[a-z]"), "[a-z]");
    }

    #[test]
    fn test_bracket_with_literal_close() {
        let result = bre_to_ere("[]abc]");
        assert!(result.contains("\\]"));
    }

    #[test]
    fn test_preprocess_s_command() {
        assert_eq!(
            preprocess_bre_script(r"s/\(foo\)/\1/"),
            "s/(foo)/\\1/"
        );
    }

    #[test]
    fn test_preprocess_address_pattern() {
        // Address patterns should be converted too
        let result = preprocess_bre_script(r"/\(foo\)/d");
        assert!(result.starts_with("/(foo)/"));
    }

    #[test]
    fn test_preprocess_ere_passthrough() {
        // ERE patterns should still work (no double-conversion needed,
        // since preprocess is only called in BRE mode)
        assert_eq!(
            preprocess_bre_script("s/[0-9]+/NUM/g"),
            // + is literal in BRE, so it gets escaped
            "s/[0-9]\\+/NUM/g"
        );
    }

    #[test]
    fn test_preprocess_y_not_converted() {
        assert_eq!(
            preprocess_bre_script("y/abc/ABC/"),
            "y/abc/ABC/"
        );
    }
}
