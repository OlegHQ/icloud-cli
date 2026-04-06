//! Array Expansion with Prefix/Suffix Handlers
//!
//! Handles array expansions that have adjacent text in double quotes:
//! - "${prefix}${arr[@]#pattern}${suffix}" - pattern removal with prefix/suffix
//! - "${prefix}${arr[@]/pattern/replacement}${suffix}" - pattern replacement with prefix/suffix
//! - "${prefix}${arr[@]}${suffix}" - simple array expansion with prefix/suffix
//! - "${arr[@]:-${default[@]}}" - array default/alternative values

use crate::interpreter::expansion::{
    apply_pattern_removal, format_element_list, apply_pattern_replacement_to_slice,
    get_array_elements, get_variable, is_variable_set, PatternRemovalSide,
};
use crate::interpreter::helpers::get_ifs_separator;
use crate::interpreter::InterpreterState;
use regex_lite::Regex;

/// Result type for array expansion handlers.
#[derive(Debug, Clone)]
pub struct ArrayPrefixSuffixResult {
    pub values: Vec<String>,
    pub quoted: bool,
}

impl ArrayPrefixSuffixResult {
    fn new(values: Vec<String>) -> Self {
        Self { values, quoted: true }
    }
}

/// Resolve array `array_name` into an element list, falling back to scalar if empty.
fn resolve_array(state: &InterpreterState, array_name: &str) -> Vec<String> {
    let elements = get_array_elements(state, array_name);
    if !elements.is_empty() {
        return elements.into_iter().map(|(_, v)| v).collect();
    }
    if let Some(scalar) = state.env.get(array_name) {
        vec![scalar.clone()]
    } else {
        vec![]
    }
}

/// Apply prefix and suffix to array elements.
/// For [@], prefix is joined to first element, suffix to last.
/// For [*], all elements are joined with IFS, then prefix and suffix are added.
pub fn apply_prefix_suffix_to_array(
    state: &InterpreterState,
    array_name: &str,
    is_star: bool,
    prefix: &str,
    suffix: &str,
) -> ArrayPrefixSuffixResult {
    let values = resolve_array(state, array_name);
    ArrayPrefixSuffixResult::new(format_element_list(values, is_star, prefix, suffix, &state.env))
}

/// Apply pattern removal with prefix/suffix to array elements.
pub fn apply_pattern_removal_with_prefix_suffix(
    state: &InterpreterState,
    array_name: &str,
    is_star: bool,
    prefix: &str,
    suffix: &str,
    regex_str: &str,
    side: PatternRemovalSide,
    greedy: bool,
) -> ArrayPrefixSuffixResult {
    let values = resolve_array(state, array_name);
    let processed: Vec<String> = values
        .iter()
        .map(|v| apply_pattern_removal(v, regex_str, side, greedy))
        .collect();
    ArrayPrefixSuffixResult::new(format_element_list(processed, is_star, prefix, suffix, &state.env))
}

/// Apply pattern replacement with prefix/suffix to array elements.
pub fn apply_pattern_replacement_with_prefix_suffix(
    state: &InterpreterState,
    array_name: &str,
    is_star: bool,
    prefix: &str,
    suffix: &str,
    regex_pattern: &str,
    replacement: &str,
    replace_all: bool,
) -> ArrayPrefixSuffixResult {
    let values = resolve_array(state, array_name);
    let processed = apply_pattern_replacement_to_slice(&values, regex_pattern, replacement, replace_all, false, false);
    ArrayPrefixSuffixResult::new(format_element_list(processed, is_star, prefix, suffix, &state.env))
}

/// Handle array default value expansion.
/// Returns the default array elements if the main array is unset/empty.
pub fn handle_array_default_value(
    state: &InterpreterState,
    array_name: &str,
    is_star: bool,
    default_array_name: &str,
    default_is_star: bool,
    check_empty: bool,
    use_alternative: bool,
) -> Option<ArrayPrefixSuffixResult> {
    let elements = get_array_elements(state, array_name);
    let is_set = !elements.is_empty() || state.env.contains_key(array_name);
    let is_empty =
        elements.is_empty() || (elements.len() == 1 && elements.iter().all(|(_, v)| v.is_empty()));

    let should_use_alternate = if use_alternative {
        is_set && !(check_empty && is_empty)
    } else {
        !is_set || (check_empty && is_empty)
    };

    if !should_use_alternate {
        if !elements.is_empty() {
            let values: Vec<String> = elements.into_iter().map(|(_, v)| v).collect();
            return Some(ArrayPrefixSuffixResult::new(
                format_element_list(values, is_star, "", "", &state.env)
            ));
        }
        if let Some(scalar_value) = state.env.get(array_name) {
            return Some(ArrayPrefixSuffixResult::new(vec![scalar_value.clone()]));
        }
        return Some(ArrayPrefixSuffixResult::new(vec![]));
    }

    // Use the default array
    let default_elements = get_array_elements(state, default_array_name);
    if !default_elements.is_empty() {
        let values: Vec<String> = default_elements.into_iter().map(|(_, v)| v).collect();
        return Some(ArrayPrefixSuffixResult::new(
            format_element_list(values, default_is_star || is_star, "", "", &state.env)
        ));
    }

    if let Some(scalar_value) = state.env.get(default_array_name) {
        return Some(ArrayPrefixSuffixResult::new(vec![scalar_value.clone()]));
    }

    Some(ArrayPrefixSuffixResult::new(vec![]))
}

/// Handle scalar variable default value with array default.
pub fn handle_scalar_default_with_array(
    state: &InterpreterState,
    var_name: &str,
    default_array_name: &str,
    default_is_star: bool,
    check_empty: bool,
    use_alternative: bool,
) -> Option<ArrayPrefixSuffixResult> {
    let is_set = is_variable_set(state, var_name);
    let var_value = get_variable(state, var_name);
    let is_empty = var_value.is_empty();

    let should_use_alternate = if use_alternative {
        is_set && !(check_empty && is_empty)
    } else {
        !is_set || (check_empty && is_empty)
    };

    if !should_use_alternate {
        return Some(ArrayPrefixSuffixResult::new(vec![var_value]));
    }

    let default_elements = get_array_elements(state, default_array_name);
    if !default_elements.is_empty() {
        let values: Vec<String> = default_elements.into_iter().map(|(_, v)| v).collect();
        return Some(ArrayPrefixSuffixResult::new(
            format_element_list(values, default_is_star, "", "", &state.env)
        ));
    }

    if let Some(scalar_value) = state.env.get(default_array_name) {
        return Some(ArrayPrefixSuffixResult::new(vec![scalar_value.clone()]));
    }

    Some(ArrayPrefixSuffixResult::new(vec![]))
}

/// Check if array assign default should be applied.
/// Returns (should_assign, current_values).
pub fn check_array_assign_default(
    state: &InterpreterState,
    array_name: &str,
    is_star: bool,
    check_empty: bool,
) -> (bool, ArrayPrefixSuffixResult) {
    let elements = get_array_elements(state, array_name);
    let is_set = !elements.is_empty() || state.env.contains_key(array_name);
    let is_empty =
        elements.is_empty() || (elements.len() == 1 && elements.iter().all(|(_, v)| v.is_empty()));

    let should_assign = !is_set || (check_empty && is_empty);

    let values: Vec<String> = elements.into_iter().map(|(_, v)| v).collect();
    let result = ArrayPrefixSuffixResult::new(
        format_element_list(values, is_star, "", "", &state.env)
    );

    (should_assign, result)
}

/// Parse array subscript from parameter name.
/// Returns (array_name, is_star) if it matches, None otherwise.
pub fn parse_array_subscript(parameter: &str) -> Option<(String, bool)> {
    let re = Regex::new(r"^([a-zA-Z_][a-zA-Z0-9_]*)\[([@*])\]$").ok()?;
    let caps = re.captures(parameter)?;
    let array_name = caps.get(1)?.as_str().to_string();
    let is_star = caps.get(2)?.as_str() == "*";
    Some((array_name, is_star))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::expansion::pattern_to_regex;
    use std::collections::HashMap;

    fn make_state() -> InterpreterState {
        let mut env = HashMap::new();
        env.insert("arr_0".to_string(), "hello".to_string());
        env.insert("arr_1".to_string(), "world".to_string());
        env.insert("arr_2".to_string(), "foo".to_string());
        InterpreterState {
            env,
            ..Default::default()
        }
    }

    #[test]
    fn test_prefix_suffix_at() {
        let state = make_state();
        let result = apply_prefix_suffix_to_array(&state, "arr", false, "pre-", "-suf");
        assert_eq!(result.values, vec!["pre-hello", "world", "foo-suf"]);
    }

    #[test]
    fn test_prefix_suffix_star() {
        let state = make_state();
        let result = apply_prefix_suffix_to_array(&state, "arr", true, "pre-", "-suf");
        assert_eq!(result.values, vec!["pre-hello world foo-suf"]);
    }

    #[test]
    fn test_prefix_suffix_single_element() {
        let mut state = make_state();
        state.env.clear();
        state.env.insert("single_0".to_string(), "only".to_string());
        let result = apply_prefix_suffix_to_array(&state, "single", false, "pre-", "-suf");
        assert_eq!(result.values, vec!["pre-only-suf"]);
    }

    #[test]
    fn test_prefix_suffix_empty_array() {
        let state = InterpreterState {
            env: HashMap::new(),
            ..Default::default()
        };
        let result = apply_prefix_suffix_to_array(&state, "empty", false, "pre-", "-suf");
        assert_eq!(result.values, vec!["pre--suf"]);
    }

    #[test]
    fn test_pattern_removal_with_prefix_suffix() {
        let state = make_state();
        let regex = pattern_to_regex("h*o", false, false);
        let result = apply_pattern_removal_with_prefix_suffix(
            &state,
            "arr",
            false,
            "pre-",
            "-suf",
            &regex,
            PatternRemovalSide::Prefix,
            false,
        );
        assert_eq!(result.values, vec!["pre-", "world", "foo-suf"]);
    }

    #[test]
    fn test_check_array_assign_default_unset() {
        let state = InterpreterState {
            env: HashMap::new(),
            ..Default::default()
        };
        let (should_assign, result) = check_array_assign_default(&state, "unset", false, false);
        assert!(should_assign);
        assert!(result.values.is_empty());
    }

    #[test]
    fn test_check_array_assign_default_set() {
        let state = make_state();
        let (should_assign, result) = check_array_assign_default(&state, "arr", false, false);
        assert!(!should_assign);
        assert_eq!(result.values, vec!["hello", "world", "foo"]);
    }

    #[test]
    fn test_parse_array_subscript() {
        assert_eq!(
            parse_array_subscript("arr[@]"),
            Some(("arr".to_string(), false))
        );
        assert_eq!(
            parse_array_subscript("arr[*]"),
            Some(("arr".to_string(), true))
        );
        assert_eq!(parse_array_subscript("arr"), None);
        assert_eq!(parse_array_subscript("arr[0]"), None);
    }
}
