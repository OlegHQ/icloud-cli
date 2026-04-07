//! Assignment Expansion Helpers
//!
//! Handles expansion of assignment arguments for local/declare/typeset builtins.
//! - Array assignments: name=(elem1 elem2 ...)
//! - Scalar assignments: name=value, name+=value, name[index]=value

use brush_parser::ast as bast;

use crate::interpreter::types::InterpreterContext;

/// Check if a Word represents an array assignment (name=(...)) and expand it
/// while preserving quote structure for elements.
/// Returns the expanded string like "name=(elem1 elem2 ...)" or None if not an array assignment.
pub fn expand_local_array_assignment(
    ctx: &mut InterpreterContext,
    word: &bast::Word,
) -> Option<String> {
    let text = &word.value;

    // Check for array assignment pattern: name=(...)
    let array_re = regex_lite::Regex::new(r"^([a-zA-Z_][a-zA-Z0-9_]*)=\(").ok()?;
    let captures = array_re.captures(text)?;

    if !text.ends_with(')') {
        return None;
    }

    let name = captures.get(1)?.as_str();

    // Extract the content between =( and )
    let eq_paren_pos = text.find("=(")?;
    let inner = &text[eq_paren_pos + 2..text.len() - 1];

    // Parse array elements, respecting quotes
    let elements = parse_array_elements(ctx, inner);

    // Build result string — re-quote empty elements to preserve them
    let quoted_elements: Vec<String> = elements
        .iter()
        .map(|elem| {
            if elem.is_empty() {
                "''".to_string()
            } else {
                elem.clone()
            }
        })
        .collect();

    Some(format!("{}=({})", name, quoted_elements.join(" ")))
}

/// Parse array elements from the inner content of an array assignment,
/// respecting single quotes, double quotes, and escapes.
fn parse_array_elements(ctx: &InterpreterContext, inner: &str) -> Vec<String> {
    let mut elements: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = inner.chars().peekable();
    let mut has_content = false;

    while let Some(ch) = chars.next() {
        match ch {
            // Whitespace splits elements (outside quotes)
            ' ' | '\t' | '\n' => {
                if has_content || !current.is_empty() {
                    elements.push(current.clone());
                    current.clear();
                    has_content = false;
                }
            }
            // Single-quoted string
            '\'' => {
                has_content = true;
                while let Some(qch) = chars.next() {
                    if qch == '\'' {
                        break;
                    }
                    current.push(qch);
                }
            }
            // Double-quoted string
            '"' => {
                has_content = true;
                while let Some(qch) = chars.next() {
                    if qch == '"' {
                        break;
                    }
                    if qch == '\\' {
                        if let Some(escaped) = chars.next() {
                            current.push(escaped);
                        }
                    } else if qch == '$' {
                        // Variable expansion inside double quotes
                        let var_name = read_variable_name(&mut chars);
                        if !var_name.is_empty() {
                            if let Some(val) = ctx.state.env.get(&var_name) {
                                current.push_str(val);
                            }
                        } else {
                            current.push('$');
                        }
                    } else {
                        current.push(qch);
                    }
                }
            }
            // Escape
            '\\' => {
                has_content = true;
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            // Variable expansion
            '$' => {
                let var_name = read_variable_name(&mut chars);
                if !var_name.is_empty() {
                    if let Some(val) = ctx.state.env.get(&var_name) {
                        current.push_str(val);
                    }
                } else {
                    current.push('$');
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if has_content || !current.is_empty() {
        elements.push(current);
    }

    elements
}

/// Read a simple variable name from a char iterator (after '$').
fn read_variable_name(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    // Handle ${...} form
    if chars.peek() == Some(&'{') {
        chars.next(); // consume '{'
        let mut name = String::new();
        for ch in chars.by_ref() {
            if ch == '}' {
                break;
            }
            name.push(ch);
        }
        return name;
    }

    // Simple $name form
    let mut name = String::new();
    while let Some(&ch) = chars.peek() {
        if ch.is_alphanumeric() || ch == '_' {
            name.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    name
}

/// Check if a Word represents a scalar assignment (name=value, name+=value, or name[index]=value)
/// and expand it WITHOUT glob expansion on the value part.
/// Returns the expanded string like "name=expanded_value" or None if not a scalar assignment.
pub fn expand_scalar_assignment_arg(
    ctx: &mut InterpreterContext,
    word: &bast::Word,
) -> Option<String> {
    let text = &word.value;

    let var_re = regex_lite::Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*$").ok()?;
    let array_re = regex_lite::Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*\[[^\]]+\]$").ok()?;

    // Check for += first
    if let Some(append_idx) = text.find("+=") {
        let before = &text[..append_idx];
        if var_re.is_match(before) || array_re.is_match(before) {
            let value_part = &text[append_idx + 2..];
            let expanded_value = expand_value_simple(ctx, value_part);
            return Some(format!("{}+={}", before, expanded_value));
        }
    }

    // Check for regular =
    if let Some(eq_idx) = text.find('=') {
        // Make sure this isn't part of +=
        if eq_idx == 0 || text.as_bytes()[eq_idx - 1] != b'+' {
            let before = &text[..eq_idx];
            if var_re.is_match(before) || array_re.is_match(before) {
                let value_part = &text[eq_idx + 1..];
                // Check if this is an array assignment (value starts with '(')
                if value_part.starts_with('(') {
                    return None;
                }
                let expanded_value = expand_value_simple(ctx, value_part);
                return Some(format!("{}={}", before, expanded_value));
            }
        }
    }

    None
}

/// Simple value expansion for assignment values.
/// Handles $VAR, ${VAR}, single quotes, double quotes, and escapes.
fn expand_value_simple(ctx: &InterpreterContext, value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                // Single-quoted: literal content
                while let Some(qch) = chars.next() {
                    if qch == '\'' {
                        break;
                    }
                    result.push(qch);
                }
            }
            '"' => {
                // Double-quoted: expand variables
                while let Some(qch) = chars.next() {
                    if qch == '"' {
                        break;
                    }
                    if qch == '\\' {
                        if let Some(escaped) = chars.next() {
                            result.push(escaped);
                        }
                    } else if qch == '$' {
                        let var_name = read_variable_name(&mut chars);
                        if !var_name.is_empty() {
                            if let Some(val) = ctx.state.env.get(&var_name) {
                                result.push_str(val);
                            }
                        } else {
                            result.push('$');
                        }
                    } else {
                        result.push(qch);
                    }
                }
            }
            '\\' => {
                if let Some(escaped) = chars.next() {
                    result.push(escaped);
                }
            }
            '$' => {
                let var_name = read_variable_name(&mut chars);
                if !var_name.is_empty() {
                    if let Some(val) = ctx.state.env.get(&var_name) {
                        result.push_str(val);
                    }
                } else {
                    result.push('$');
                }
            }
            _ => {
                result.push(ch);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::types::{ExecutionLimits, InterpreterState};

    fn make_ctx() -> (InterpreterState, ExecutionLimits) {
        (InterpreterState::default(), ExecutionLimits::default())
    }

    fn make_word(value: &str) -> bast::Word {
        bast::Word {
            value: value.to_string(),
            loc: None,
        }
    }

    #[test]
    fn test_expand_scalar_assignment_simple() {
        let (mut state, limits) = make_ctx();
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        let word = make_word("foo=bar");
        let result = expand_scalar_assignment_arg(&mut ctx, &word);
        assert_eq!(result, Some("foo=bar".to_string()));
    }

    #[test]
    fn test_expand_scalar_assignment_with_variable() {
        let (mut state, limits) = make_ctx();
        state.env.insert("x".to_string(), "hello".to_string());
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        let word = make_word("foo=$x");
        let result = expand_scalar_assignment_arg(&mut ctx, &word);
        assert_eq!(result, Some("foo=hello".to_string()));
    }

    #[test]
    fn test_not_an_assignment() {
        let (mut state, limits) = make_ctx();
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        let word = make_word("echo");
        let result = expand_scalar_assignment_arg(&mut ctx, &word);
        assert_eq!(result, None);
    }

    #[test]
    fn test_array_assignment_simple() {
        let (mut state, limits) = make_ctx();
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        let word = make_word("arr=(a b c)");
        let result = expand_local_array_assignment(&mut ctx, &word);
        assert_eq!(result, Some("arr=(a b c)".to_string()));
    }

    #[test]
    fn test_array_assignment_empty_quoted_string() {
        let (mut state, limits) = make_ctx();
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        // arr=('' "")  -- empty single-quoted and double-quoted strings
        let word = make_word("arr=('' \"\")");
        let result = expand_local_array_assignment(&mut ctx, &word);
        assert_eq!(result, Some("arr=('' '')".to_string()));
    }

    #[test]
    fn test_array_assignment_with_variable() {
        let (mut state, limits) = make_ctx();
        state.env.insert("x".to_string(), "hello world".to_string());
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        // arr=($x) — variable expands and word-splits into two elements
        let word = make_word("arr=($x)");
        let result = expand_local_array_assignment(&mut ctx, &word);
        assert_eq!(result, Some("arr=(hello world)".to_string()));
    }

    #[test]
    fn test_array_assignment_keyed_element() {
        let (mut state, limits) = make_ctx();
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        // arr=([0]=foo [1]=bar)
        let word = make_word("arr=([0]=foo [1]=bar)");
        let result = expand_local_array_assignment(&mut ctx, &word);
        assert_eq!(result, Some("arr=([0]=foo [1]=bar)".to_string()));
    }

    #[test]
    fn test_scalar_assignment_append() {
        let (mut state, limits) = make_ctx();
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        let word = make_word("foo+=bar");
        let result = expand_scalar_assignment_arg(&mut ctx, &word);
        assert_eq!(result, Some("foo+=bar".to_string()));
    }

    #[test]
    fn test_scalar_assignment_array_index() {
        let (mut state, limits) = make_ctx();
        let mut ctx = InterpreterContext::new(&mut state, &limits);

        let word = make_word("arr[0]=val");
        let result = expand_scalar_assignment_arg(&mut ctx, &word);
        assert_eq!(result, Some("arr[0]=val".to_string()));
    }
}
