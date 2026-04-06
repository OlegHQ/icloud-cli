//! Positional Parameter Expansion Handlers
//!
//! Handles $@ and $* expansion with various operations:
//! - "${@:offset}" and "${*:offset}" - slicing
//! - "${@/pattern/replacement}" - pattern replacement
//! - "${@#pattern}" - pattern removal (strip)
//! - "$@" and "$*" with adjacent text

use crate::interpreter::expansion::{
    apply_pattern_removal, apply_pattern_replacement_to_slice, format_element_list,
    slice_elements, PatternRemovalSide,
};
use crate::interpreter::InterpreterState;

/// Result type for positional parameter expansion handlers.
#[derive(Debug, Clone)]
pub struct PositionalExpansionResult {
    pub values: Vec<String>,
    pub quoted: bool,
}

impl PositionalExpansionResult {
    fn new(values: Vec<String>) -> Self {
        Self { values, quoted: true }
    }
}

/// Get positional parameters from state
pub fn get_positional_params(state: &InterpreterState) -> Vec<String> {
    let num_params: i32 = state.env.get("#").and_then(|s| s.parse().ok()).unwrap_or(0);
    (1..=num_params)
        .map(|i| state.env.get(&i.to_string()).cloned().unwrap_or_default())
        .collect()
}

/// Apply positional parameter slicing.
/// offset and length should be pre-evaluated (arithmetic evaluation requires async).
pub fn apply_positional_slicing(
    state: &InterpreterState,
    is_star: bool,
    prefix: &str,
    suffix: &str,
    offset: i64,
    length: Option<i64>,
) -> PositionalExpansionResult {
    let all_params = get_positional_params(state);

    let sliced_params: Vec<String> = if offset <= 0 {
        // offset 0: include $0 at position 0
        let shell_name = state.env.get("0").cloned().unwrap_or_else(|| "bash".to_string());
        let mut with_zero = vec![shell_name];
        with_zero.extend(all_params);

        let computed_idx = with_zero.len() as i64 + offset;
        if computed_idx < 0 {
            vec![]
        } else {
            let start_idx = if offset < 0 { computed_idx as usize } else { 0 };
            slice_elements(&with_zero, start_idx as i64, length)
        }
    } else {
        slice_elements(&all_params, offset - 1, length)
    };

    PositionalExpansionResult::new(format_element_list(sliced_params, is_star, prefix, suffix, &state.env))
}

/// Apply pattern replacement to positional parameters.
/// regex_pattern and replacement should be pre-expanded.
pub fn apply_positional_pattern_replacement(
    state: &InterpreterState,
    is_star: bool,
    prefix: &str,
    suffix: &str,
    regex_pattern: &str,
    replacement: &str,
    replace_all: bool,
    anchor_start: bool,
    anchor_end: bool,
) -> PositionalExpansionResult {
    let params = get_positional_params(state);
    let replaced = apply_pattern_replacement_to_slice(
        &params, regex_pattern, replacement, replace_all, anchor_start, anchor_end,
    );
    PositionalExpansionResult::new(format_element_list(replaced, is_star, prefix, suffix, &state.env))
}

/// Apply pattern removal to positional parameters.
/// regex_str should be pre-expanded.
pub fn apply_positional_pattern_removal(
    state: &InterpreterState,
    is_star: bool,
    prefix: &str,
    suffix: &str,
    regex_str: &str,
    side: PatternRemovalSide,
    greedy: bool,
) -> PositionalExpansionResult {
    let params = get_positional_params(state);
    let stripped: Vec<String> = params
        .iter()
        .map(|p| apply_pattern_removal(p, regex_str, side, greedy))
        .collect();
    PositionalExpansionResult::new(format_element_list(stripped, is_star, prefix, suffix, &state.env))
}

/// Handle simple "$@" and "$*" expansion with prefix/suffix.
pub fn apply_simple_positional_expansion(
    state: &InterpreterState,
    is_star: bool,
    prefix: &str,
    suffix: &str,
) -> PositionalExpansionResult {
    let num_params: i32 = state.env.get("#").and_then(|s| s.parse().ok()).unwrap_or(0);

    if num_params == 0 {
        // "$*" with no params -> one empty word (prefix + suffix)
        // "$@" with no params -> no words (unless there's prefix/suffix)
        let combined = format!("{}{}", prefix, suffix);
        return if is_star {
            PositionalExpansionResult::new(vec![combined])
        } else if combined.is_empty() {
            PositionalExpansionResult::new(vec![])
        } else {
            PositionalExpansionResult::new(vec![combined])
        };
    }

    let params = get_positional_params(state);
    PositionalExpansionResult::new(format_element_list(params, is_star, prefix, suffix, &state.env))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::expansion::pattern_to_regex;
    use std::collections::HashMap;

    fn make_state_with_params(params: &[&str]) -> InterpreterState {
        let mut env = HashMap::new();
        env.insert("#".to_string(), params.len().to_string());
        env.insert("0".to_string(), "bash".to_string());
        for (i, p) in params.iter().enumerate() {
            env.insert((i + 1).to_string(), p.to_string());
        }
        InterpreterState {
            env,
            ..Default::default()
        }
    }

    #[test]
    fn test_get_positional_params() {
        let state = make_state_with_params(&["a", "b", "c"]);
        let params = get_positional_params(&state);
        assert_eq!(params, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_simple_at_expansion() {
        let state = make_state_with_params(&["hello", "world"]);
        let result = apply_simple_positional_expansion(&state, false, "pre-", "-suf");
        assert_eq!(result.values, vec!["pre-hello", "world-suf"]);
    }

    #[test]
    fn test_simple_star_expansion() {
        let state = make_state_with_params(&["hello", "world"]);
        let result = apply_simple_positional_expansion(&state, true, "pre-", "-suf");
        assert_eq!(result.values, vec!["pre-hello world-suf"]);
    }

    #[test]
    fn test_simple_at_expansion_empty() {
        let state = make_state_with_params(&[]);
        let result = apply_simple_positional_expansion(&state, false, "pre-", "-suf");
        assert_eq!(result.values, vec!["pre--suf"]);
    }

    #[test]
    fn test_simple_star_expansion_empty() {
        let state = make_state_with_params(&[]);
        let result = apply_simple_positional_expansion(&state, true, "", "");
        assert_eq!(result.values, vec![""]);
    }

    #[test]
    fn test_slicing_offset_positive() {
        let state = make_state_with_params(&["a", "b", "c", "d"]);
        let result = apply_positional_slicing(&state, false, "", "", 2, None);
        assert_eq!(result.values, vec!["b", "c", "d"]);
    }

    #[test]
    fn test_slicing_offset_with_length() {
        let state = make_state_with_params(&["a", "b", "c", "d"]);
        let result = apply_positional_slicing(&state, false, "", "", 2, Some(2));
        assert_eq!(result.values, vec!["b", "c"]);
    }

    #[test]
    fn test_slicing_offset_zero() {
        let state = make_state_with_params(&["a", "b", "c"]);
        let result = apply_positional_slicing(&state, false, "", "", 0, Some(2));
        assert_eq!(result.values, vec!["bash", "a"]);
    }

    #[test]
    fn test_pattern_replacement() {
        let state = make_state_with_params(&["hello", "world", "help"]);
        let regex = pattern_to_regex("hel", true, false);
        let result = apply_positional_pattern_replacement(
            &state, false, "", "", &regex, "HEL", false, false, false,
        );
        assert_eq!(result.values, vec!["HELlo", "world", "HELp"]);
    }

    #[test]
    fn test_pattern_removal() {
        let state = make_state_with_params(&["hello", "world", "help"]);
        let regex = pattern_to_regex("hel", false, false);
        let result = apply_positional_pattern_removal(
            &state,
            false,
            "",
            "",
            &regex,
            PatternRemovalSide::Prefix,
            false,
        );
        assert_eq!(result.values, vec!["lo", "world", "p"]);
    }

    #[test]
    fn test_single_param_with_prefix_suffix() {
        let state = make_state_with_params(&["only"]);
        let result = apply_simple_positional_expansion(&state, false, "pre-", "-suf");
        assert_eq!(result.values, vec!["pre-only-suf"]);
    }
}
