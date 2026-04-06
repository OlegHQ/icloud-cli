// src/commands/vfs_helpers.rs
//! VFS (virtual filesystem) helpers for command implementations.
//! Wraps the common pattern of resolving a path and reading a file with
//! a standard "No such file or directory" error on failure.

use crate::commands::errors::no_such_file;
use crate::fs::FileSystem;

/// Resolve `path` relative to `cwd` and read the file contents.
/// On error returns `Err` containing the standard error string.
pub async fn read_file_or_error(
    fs: &dyn FileSystem,
    cwd: &str,
    cmd: &str,
    path: &str,
) -> Result<String, String> {
    let full = fs.resolve_path(cwd, path);
    fs.read_file(&full)
        .await
        .map_err(|_| no_such_file(cmd, path))
}

/// Like `read_file_or_error`, but appends the error to `stderr` and sets
/// `exit_code` to 1 when the file cannot be read.  Returns `None` on error.
pub async fn read_file_accumulate_errors(
    fs: &dyn FileSystem,
    cwd: &str,
    cmd: &str,
    path: &str,
    stderr: &mut String,
    exit_code: &mut i32,
) -> Option<String> {
    match read_file_or_error(fs, cwd, cmd, path).await {
        Ok(content) => Some(content),
        Err(msg) => {
            stderr.push_str(&msg);
            *exit_code = 1;
            None
        }
    }
}
