//! Shared regex/pattern operations used across expansion modules.
//!
//! Centralises the pattern replacement logic that was duplicated in
//! `array_prefix_suffix`, `positional_params`, and `unquoted_expansion`.

use regex_lite::Regex;

/// Apply a regex-based pattern replacement to a single value.
///
/// `anchor_start` prepends `^`, `anchor_end` appends `$`.
/// When both are `false` the pattern is applied unanchored.
pub fn apply_pattern_replacement(
    value: &str,
    regex_pattern: &str,
    replacement: &str,
    replace_all: bool,
    anchor_start: bool,
    anchor_end: bool,
) -> String {
    let final_pattern = if anchor_start {
        format!("^{}", regex_pattern)
    } else if anchor_end {
        format!("{}$", regex_pattern)
    } else {
        regex_pattern.to_string()
    };

    match Regex::new(&final_pattern) {
        Ok(re) => {
            if replace_all {
                re.replace_all(value, replacement).to_string()
            } else {
                re.replace(value, replacement).to_string()
            }
        }
        Err(_) => value.to_string(),
    }
}

/// Apply `apply_pattern_replacement` to every element of a slice.
pub fn apply_pattern_replacement_to_slice(
    values: &[String],
    regex_pattern: &str,
    replacement: &str,
    replace_all: bool,
    anchor_start: bool,
    anchor_end: bool,
) -> Vec<String> {
    values
        .iter()
        .map(|v| {
            apply_pattern_replacement(v, regex_pattern, replacement, replace_all, anchor_start, anchor_end)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_replace() {
        assert_eq!(
            apply_pattern_replacement("hello world", "o", "0", false, false, false),
            "hell0 world"
        );
    }

    #[test]
    fn test_replace_all() {
        assert_eq!(
            apply_pattern_replacement("hello", "l", "L", true, false, false),
            "heLLo"
        );
    }

    #[test]
    fn test_anchor_start() {
        assert_eq!(
            apply_pattern_replacement("hello", "h", "H", false, true, false),
            "Hello"
        );
        assert_eq!(
            apply_pattern_replacement("world", "h", "H", false, true, false),
            "world"
        );
    }

    #[test]
    fn test_anchor_end() {
        assert_eq!(
            apply_pattern_replacement("hello", "o", "0", false, false, true),
            "hell0"
        );
    }

    #[test]
    fn test_invalid_regex_passthrough() {
        // Invalid regex should return the original value unchanged
        assert_eq!(
            apply_pattern_replacement("hello", "[invalid", "x", false, false, false),
            "hello"
        );
    }

    #[test]
    fn test_slice() {
        let values = vec!["hello".to_string(), "world".to_string(), "help".to_string()];
        assert_eq!(
            apply_pattern_replacement_to_slice(&values, "hel", "HEL", false, false, false),
            vec!["HELlo", "world", "HELp"]
        );
    }
}
