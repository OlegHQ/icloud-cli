// src/interpreter/helpers/identifier.rs
//! Identifier validation for shell variable names.

/// Return true if `name` is a valid shell identifier:
/// starts with an ASCII letter or underscore, followed by zero or more
/// ASCII alphanumeric characters or underscores.
pub fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_identifiers() {
        assert!(is_valid_identifier("FOO"));
        assert!(is_valid_identifier("_foo"));
        assert!(is_valid_identifier("foo123"));
        assert!(is_valid_identifier("_123"));
    }

    #[test]
    fn test_invalid_identifiers() {
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123foo"));
        assert!(!is_valid_identifier("foo-bar"));
        assert!(!is_valid_identifier("foo bar"));
    }
}
