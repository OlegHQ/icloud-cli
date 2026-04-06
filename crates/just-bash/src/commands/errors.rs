// src/commands/errors.rs
//! Common error message helpers for command implementations.
//! Each function produces the standard POSIX-style error string with a trailing newline.

pub fn no_such_file(cmd: &str, path: &str) -> String {
    format!("{}: {}: No such file or directory\n", cmd, path)
}

pub fn is_a_directory(cmd: &str, path: &str) -> String {
    format!("{}: {}: Is a directory\n", cmd, path)
}

pub fn permission_denied(cmd: &str, path: &str) -> String {
    format!("{}: {}: Permission denied\n", cmd, path)
}

pub fn not_a_directory(cmd: &str, path: &str) -> String {
    format!("{}: {}: Not a directory\n", cmd, path)
}
