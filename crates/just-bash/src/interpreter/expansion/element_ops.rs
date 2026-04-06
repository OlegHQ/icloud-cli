//! Shared element operations for arrays and positional parameters.
//!
//! Both `${arr[@]#pat}` and `${@#pat}` apply the same per-element operations
//! followed by the same at/star formatting.  This module contains the shared
//! primitives so neither `array_prefix_suffix` nor `positional_params` needs
//! its own copy.

use crate::interpreter::helpers::get_ifs_separator;
use std::collections::HashMap;

/// Format a slice of already-processed elements with an at/star style and
/// optional outer prefix/suffix.
///
/// * `is_star` – `true` → join all elements with IFS into one word
/// * `prefix`/`suffix` – joined to the first/last element respectively
pub fn format_element_list(
    values: Vec<String>,
    is_star: bool,
    prefix: &str,
    suffix: &str,
    env: &HashMap<String, String>,
) -> Vec<String> {
    if values.is_empty() {
        let combined = format!("{}{}", prefix, suffix);
        if is_star {
            return vec![combined];
        }
        return if combined.is_empty() { vec![] } else { vec![combined] };
    }

    if is_star {
        let sep = get_ifs_separator(env);
        return vec![format!("{}{}{}", prefix, values.join(sep), suffix)];
    }

    if values.len() == 1 {
        return vec![format!("{}{}{}", prefix, values[0], suffix)];
    }

    let mut result = Vec::with_capacity(values.len());
    result.push(format!("{}{}", prefix, values[0]));
    for v in &values[1..values.len() - 1] {
        result.push(v.clone());
    }
    result.push(format!("{}{}", values[values.len() - 1], suffix));
    result
}

/// Slice a list of elements by offset/length, mirroring bash `${arr[@]:off:len}`.
pub fn slice_elements(values: &[String], offset: i64, length: Option<i64>) -> Vec<String> {
    let start = if offset < 0 {
        let computed = values.len() as i64 + offset;
        computed.max(0) as usize
    } else {
        offset as usize
    };

    if start >= values.len() {
        return vec![];
    }

    let end = match length {
        Some(len) if len < 0 => {
            let computed = values.len() as i64 + len;
            if computed < start as i64 {
                return vec![];
            }
            computed as usize
        }
        Some(len) => (start + len as usize).min(values.len()),
        None => values.len(),
    };

    values[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_space() -> HashMap<String, String> {
        HashMap::new() // IFS defaults to space
    }

    #[test]
    fn test_format_at_multiple() {
        let values = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(
            format_element_list(values, false, "pre-", "-suf", &env_space()),
            vec!["pre-a", "b", "c-suf"]
        );
    }

    #[test]
    fn test_format_star_multiple() {
        let values = vec!["a".to_string(), "b".to_string()];
        // Default IFS is space, so join is "a b"; with prefix "p" and suffix "s": "pa bs"
        assert_eq!(
            format_element_list(values, true, "p", "s", &env_space()),
            vec!["pa bs"]
        );
    }

    #[test]
    fn test_format_single() {
        let values = vec!["only".to_string()];
        assert_eq!(
            format_element_list(values, false, "p-", "-s", &env_space()),
            vec!["p-only-s"]
        );
    }

    #[test]
    fn test_format_empty_at() {
        assert_eq!(
            format_element_list(vec![], false, "", "", &env_space()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_format_empty_star() {
        assert_eq!(
            format_element_list(vec![], true, "", "", &env_space()),
            vec![""]
        );
    }

    #[test]
    fn test_slice_basic() {
        let v = vec!["a", "b", "c", "d"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        assert_eq!(slice_elements(&v, 1, Some(2)), vec!["b", "c"]);
    }

    #[test]
    fn test_slice_no_length() {
        let v = vec!["a", "b", "c"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        assert_eq!(slice_elements(&v, 1, None), vec!["b", "c"]);
    }

    #[test]
    fn test_slice_negative_offset() {
        let v = vec!["a", "b", "c"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        assert_eq!(slice_elements(&v, -1, None), vec!["c"]);
    }

    #[test]
    fn test_slice_out_of_bounds() {
        let v = vec!["a".to_string()];
        assert_eq!(slice_elements(&v, 5, None), Vec::<String>::new());
    }
}
