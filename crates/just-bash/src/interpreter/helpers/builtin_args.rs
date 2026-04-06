//! Shared argument parsing helpers for builtin commands.

/// Parse a numeric exit-code argument that must be an integer in the range
/// representable by i32 (modulo-256 wrapping applied by callers).
///
/// Returns `Ok(Some(n))` if one valid argument was given,
/// `Ok(None)` if no argument was given,
/// or `Err(message)` if the argument is invalid.
pub fn parse_numeric_arg(cmd_name: &str, args: &[String]) -> Result<Option<i32>, String> {
    match args.first() {
        None => Ok(None),
        Some(arg) => {
            if arg.is_empty() || !arg.chars().all(|c| c.is_ascii_digit() || c == '-') {
                Err(format!(
                    "bash: {}: {}: numeric argument required\n",
                    cmd_name, arg
                ))
            } else {
                arg.parse::<i32>()
                    .map(Some)
                    .map_err(|_| format!("bash: {}: {}: numeric argument required\n", cmd_name, arg))
            }
        }
    }
}

/// Apply bash's modulo-256 exit-code wrapping.
pub fn wrap_exit_code(n: i32) -> i32 {
    ((n % 256) + 256) % 256
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_numeric_arg_none() {
        assert_eq!(parse_numeric_arg("exit", &[]), Ok(None));
    }

    #[test]
    fn test_parse_numeric_arg_valid() {
        assert_eq!(
            parse_numeric_arg("exit", &["42".to_string()]),
            Ok(Some(42))
        );
    }

    #[test]
    fn test_parse_numeric_arg_negative() {
        assert_eq!(
            parse_numeric_arg("exit", &["-1".to_string()]),
            Ok(Some(-1))
        );
    }

    #[test]
    fn test_parse_numeric_arg_invalid() {
        assert!(parse_numeric_arg("exit", &["abc".to_string()]).is_err());
    }

    #[test]
    fn test_wrap_exit_code() {
        assert_eq!(wrap_exit_code(0), 0);
        assert_eq!(wrap_exit_code(255), 255);
        assert_eq!(wrap_exit_code(256), 0);
        assert_eq!(wrap_exit_code(300), 44);
        assert_eq!(wrap_exit_code(-1), 255);
    }
}
