//! Array helper functions for the interpreter.
//!
//! Provides utilities for working with bash arrays (both indexed and associative).

use brush_parser::ast as bast;
use std::collections::HashMap;

/// Get all indices of an array, sorted in ascending order.
/// Arrays are stored as `name_0`, `name_1`, etc. in the environment.
pub fn get_array_indices(env: &HashMap<String, String>, array_name: &str) -> Vec<i64> {
    let prefix = format!("{}_", array_name);
    let mut indices: Vec<i64> = Vec::new();

    for key in env.keys() {
        if let Some(index_str) = key.strip_prefix(&prefix) {
            if let Ok(index) = index_str.parse::<i64>() {
                // Only include numeric indices (not __length or other metadata)
                if index.to_string() == index_str {
                    indices.push(index);
                }
            }
        }
    }

    indices.sort();
    indices
}

/// Clear all elements of an array from the environment.
pub fn clear_array(env: &mut HashMap<String, String>, array_name: &str) {
    let prefix = format!("{}_", array_name);
    let keys_to_remove: Vec<String> = env
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .cloned()
        .collect();

    for key in keys_to_remove {
        env.remove(&key);
    }
}

/// Get all keys of an associative array.
/// For associative arrays, keys are stored as `name_key` where key is a string.
pub fn get_assoc_array_keys(env: &HashMap<String, String>, array_name: &str) -> Vec<String> {
    let prefix = format!("{}_", array_name);
    let metadata_suffix = format!("{}__length", array_name);
    let mut keys: Vec<String> = Vec::new();

    for env_key in env.keys() {
        // Skip the metadata entry (name__length)
        if env_key == &metadata_suffix {
            continue;
        }
        if let Some(key) = env_key.strip_prefix(&prefix) {
            // Skip if the key itself starts with underscore (would be part of metadata pattern)
            if key.starts_with("_length") {
                continue;
            }
            keys.push(key.to_string());
        }
    }

    keys.sort();
    keys
}

/// Remove surrounding quotes from a key string.
/// Handles 'key' and "key" → key
pub fn unquote_key(key: &str) -> &str {
    if key.len() >= 2
        && ((key.starts_with('\'') && key.ends_with('\''))
            || (key.starts_with('"') && key.ends_with('"')))
    {
        &key[1..key.len() - 1]
    } else {
        key
    }
}

/// Parsed keyed array element from a Word like [key]=value or [key]+=value.
#[derive(Debug, Clone)]
pub struct ParsedKeyedElement {
    pub key: String,
    pub value: String,
    pub append: bool,
}

/// Parse a keyed array element from a Word like [key]=value or [key]+=value.
/// Since `bast::Word` is a string wrapper, this parses the pattern from the string value.
/// Returns None if not a keyed element pattern.
pub fn parse_keyed_element_from_word(word: &bast::Word) -> Option<ParsedKeyedElement> {
    let s = &word.value;

    // Must start with '['
    if !s.starts_with('[') {
        return None;
    }

    // Find the closing bracket - handle nested brackets
    let mut depth = 0;
    let mut bracket_end = None;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    bracket_end = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    let bracket_end = bracket_end?;

    // Extract key (between [ and ])
    let mut key = s[1..bracket_end].to_string();

    // After the ']', expect '=' or '+='
    let after_bracket = &s[bracket_end + 1..];
    let (append, value_str) = if let Some(rest) = after_bracket.strip_prefix("+=") {
        (true, rest)
    } else if let Some(rest) = after_bracket.strip_prefix('=') {
        (false, rest)
    } else {
        return None;
    };

    // Remove surrounding quotes from key
    key = unquote_key(&key).to_string();

    Some(ParsedKeyedElement {
        key,
        value: value_str.to_string(),
        append,
    })
}

/// Extract literal string content from a Word.
/// Since `bast::Word` is a string wrapper, this simply returns the value.
pub fn word_to_literal_string(word: &bast::Word) -> String {
    word.value.clone()
}

/// Get an array element value.
pub fn get_array_element<'a>(
    env: &'a HashMap<String, String>,
    array_name: &str,
    index: i64,
) -> Option<&'a String> {
    let key = format!("{}_{}", array_name, index);
    env.get(&key)
}

/// Set an array element value.
pub fn set_array_element(
    env: &mut HashMap<String, String>,
    array_name: &str,
    index: i64,
    value: String,
) {
    let key = format!("{}_{}", array_name, index);
    env.insert(key, value);
}

/// Get an associative array element value.
pub fn get_assoc_array_element<'a>(
    env: &'a HashMap<String, String>,
    array_name: &str,
    key: &str,
) -> Option<&'a String> {
    let env_key = format!("{}_{}", array_name, key);
    env.get(&env_key)
}

/// Set an associative array element value.
pub fn set_assoc_array_element(
    env: &mut HashMap<String, String>,
    array_name: &str,
    key: &str,
    value: String,
) {
    let env_key = format!("{}_{}", array_name, key);
    env.insert(env_key, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_array_indices() {
        let mut env = HashMap::new();
        env.insert("arr_0".to_string(), "a".to_string());
        env.insert("arr_2".to_string(), "b".to_string());
        env.insert("arr_5".to_string(), "c".to_string());
        env.insert("other".to_string(), "x".to_string());

        let indices = get_array_indices(&env, "arr");
        assert_eq!(indices, vec![0, 2, 5]);
    }

    #[test]
    fn test_get_array_indices_empty() {
        let env = HashMap::new();
        let indices = get_array_indices(&env, "arr");
        assert!(indices.is_empty());
    }

    #[test]
    fn test_clear_array() {
        let mut env = HashMap::new();
        env.insert("arr_0".to_string(), "a".to_string());
        env.insert("arr_1".to_string(), "b".to_string());
        env.insert("other".to_string(), "x".to_string());

        clear_array(&mut env, "arr");

        assert!(!env.contains_key("arr_0"));
        assert!(!env.contains_key("arr_1"));
        assert!(env.contains_key("other"));
    }

    #[test]
    fn test_get_assoc_array_keys() {
        let mut env = HashMap::new();
        env.insert("map_foo".to_string(), "1".to_string());
        env.insert("map_bar".to_string(), "2".to_string());
        env.insert("map_baz".to_string(), "3".to_string());
        env.insert("other".to_string(), "x".to_string());

        let keys = get_assoc_array_keys(&env, "map");
        assert_eq!(keys, vec!["bar", "baz", "foo"]);
    }

    #[test]
    fn test_unquote_key() {
        assert_eq!(unquote_key("'hello'"), "hello");
        assert_eq!(unquote_key("\"world\""), "world");
        assert_eq!(unquote_key("plain"), "plain");
        assert_eq!(unquote_key("'"), "'");
    }

    #[test]
    fn test_get_set_array_element() {
        let mut env = HashMap::new();
        set_array_element(&mut env, "arr", 0, "hello".to_string());
        set_array_element(&mut env, "arr", 5, "world".to_string());

        assert_eq!(
            get_array_element(&env, "arr", 0),
            Some(&"hello".to_string())
        );
        assert_eq!(
            get_array_element(&env, "arr", 5),
            Some(&"world".to_string())
        );
        assert_eq!(get_array_element(&env, "arr", 1), None);
    }

    #[test]
    fn test_get_set_assoc_array_element() {
        let mut env = HashMap::new();
        set_assoc_array_element(&mut env, "map", "foo", "bar".to_string());
        set_assoc_array_element(&mut env, "map", "baz", "qux".to_string());

        assert_eq!(
            get_assoc_array_element(&env, "map", "foo"),
            Some(&"bar".to_string())
        );
        assert_eq!(
            get_assoc_array_element(&env, "map", "baz"),
            Some(&"qux".to_string())
        );
        assert_eq!(get_assoc_array_element(&env, "map", "missing"), None);
    }

    #[test]
    fn test_parse_keyed_element_simple() {
        let word = bast::Word {
            value: "[foo]=bar".to_string(),
            loc: None,
        };
        let parsed = parse_keyed_element_from_word(&word).unwrap();
        assert_eq!(parsed.key, "foo");
        assert_eq!(parsed.value, "bar");
        assert!(!parsed.append);
    }

    #[test]
    fn test_parse_keyed_element_append() {
        let word = bast::Word {
            value: "[foo]+=bar".to_string(),
            loc: None,
        };
        let parsed = parse_keyed_element_from_word(&word).unwrap();
        assert_eq!(parsed.key, "foo");
        assert_eq!(parsed.value, "bar");
        assert!(parsed.append);
    }

    #[test]
    fn test_parse_keyed_element_quoted_key() {
        let word = bast::Word {
            value: "[\"hello\"]=world".to_string(),
            loc: None,
        };
        let parsed = parse_keyed_element_from_word(&word).unwrap();
        assert_eq!(parsed.key, "hello");
        assert_eq!(parsed.value, "world");
    }

    #[test]
    fn test_parse_keyed_element_not_keyed() {
        let word = bast::Word {
            value: "notkeyed".to_string(),
            loc: None,
        };
        assert!(parse_keyed_element_from_word(&word).is_none());
    }

    #[test]
    fn test_parse_keyed_element_no_equals() {
        let word = bast::Word {
            value: "[key]value".to_string(),
            loc: None,
        };
        assert!(parse_keyed_element_from_word(&word).is_none());
    }

    #[test]
    fn test_word_to_literal_string() {
        let word = bast::Word {
            value: "hello world".to_string(),
            loc: None,
        };
        assert_eq!(word_to_literal_string(&word), "hello world");
    }
}
