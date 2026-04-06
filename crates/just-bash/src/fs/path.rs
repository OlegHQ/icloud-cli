//! Canonical path normalization for the virtual filesystem.
//!
//! Resolves `.`, `..`, duplicate slashes, and trailing slashes without
//! touching the real filesystem (no symlink resolution).

/// Normalize a path by resolving `.`, `..`, duplicate slashes, and trailing
/// slashes.
///
/// All results are absolute (rooted at `/`).  Relative input is treated as
/// though it were already under `/`.  Empty input maps to `"/"`.
/// `..` past the root is silently absorbed.
///
/// No symlink resolution is performed — this is purely lexical.
pub fn normalize_path(path: &str) -> String {
    if path.is_empty() {
        return "/".to_string();
    }

    let mut components: Vec<&str> = Vec::new();

    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            c => {
                components.push(c);
            }
        }
    }

    if components.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", components.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn test_root() {
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn test_absolute_simple() {
        assert_eq!(normalize_path("/foo/bar"), "/foo/bar");
    }

    #[test]
    fn test_trailing_slash() {
        assert_eq!(normalize_path("/foo/bar/"), "/foo/bar");
    }

    #[test]
    fn test_relative_becomes_absolute() {
        assert_eq!(normalize_path("foo/bar"), "/foo/bar");
    }

    #[test]
    fn test_dot_removal() {
        assert_eq!(normalize_path("/foo/./bar"), "/foo/bar");
    }

    #[test]
    fn test_dotdot_resolution() {
        assert_eq!(normalize_path("/foo/../bar"), "/bar");
        assert_eq!(normalize_path("/foo/bar/.."), "/foo");
        assert_eq!(normalize_path("/foo/bar/../baz"), "/foo/baz");
    }

    #[test]
    fn test_dotdot_at_root() {
        assert_eq!(normalize_path("/foo/bar/../.."), "/");
        assert_eq!(normalize_path("/../.."), "/");
    }

    #[test]
    fn test_double_slash() {
        assert_eq!(normalize_path("/foo//bar"), "/foo/bar");
    }
}
