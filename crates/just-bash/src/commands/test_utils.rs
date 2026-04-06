// src/commands/test_utils.rs
//
// Shared test helpers for command unit tests.
// All items are cfg(test)-only.

use crate::commands::{CommandContext, CommandResult};
use crate::fs::FileSystem;
pub use crate::fs::InMemoryFs;
pub use std::collections::HashMap;
pub use std::sync::Arc;

/// Basic context: empty stdin, cwd="/", no env.
pub fn make_ctx(args: Vec<&str>) -> CommandContext {
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: String::new(),
        cwd: "/".to_string(),
        env: HashMap::new(),
        fs: Arc::new(InMemoryFs::new()),
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Context with stdin.
pub fn make_ctx_with_stdin(args: Vec<&str>, stdin: &str) -> CommandContext {
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: stdin.to_string(),
        cwd: "/".to_string(),
        env: HashMap::new(),
        fs: Arc::new(InMemoryFs::new()),
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Context with a pre-built filesystem.
pub fn make_ctx_with_fs(args: Vec<&str>, fs: Arc<InMemoryFs>) -> CommandContext {
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: String::new(),
        cwd: "/".to_string(),
        env: HashMap::new(),
        fs,
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Context with stdin and a pre-built filesystem.
pub fn make_ctx_with_stdin_and_fs(
    args: Vec<&str>,
    stdin: &str,
    fs: Arc<InMemoryFs>,
) -> CommandContext {
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: stdin.to_string(),
        cwd: "/".to_string(),
        env: HashMap::new(),
        fs,
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Context with custom env vars.
pub fn make_ctx_with_env(args: Vec<&str>, env: HashMap<String, String>) -> CommandContext {
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: String::new(),
        cwd: "/".to_string(),
        env,
        fs: Arc::new(InMemoryFs::new()),
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Async: context with files pre-written into the filesystem.
pub async fn make_ctx_with_files(
    args: Vec<&str>,
    files: Vec<(&str, &str)>,
) -> CommandContext {
    let fs = Arc::new(InMemoryFs::new());
    for (path, content) in files {
        fs.write_file(path, content.as_bytes()).await.unwrap();
    }
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: String::new(),
        cwd: "/".to_string(),
        env: HashMap::new(),
        fs,
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Async: context with stdin and files.
pub async fn make_ctx_with_stdin_and_files(
    args: Vec<&str>,
    stdin: &str,
    files: Vec<(&str, &str)>,
) -> CommandContext {
    let fs = Arc::new(InMemoryFs::new());
    for (path, content) in files {
        fs.write_file(path, content.as_bytes()).await.unwrap();
    }
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: stdin.to_string(),
        cwd: "/".to_string(),
        env: HashMap::new(),
        fs,
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Context with stdin and custom env vars.
pub fn make_ctx_with_stdin_and_env(
    args: Vec<&str>,
    stdin: &str,
    env: HashMap<String, String>,
) -> CommandContext {
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: stdin.to_string(),
        cwd: "/".to_string(),
        env,
        fs: Arc::new(InMemoryFs::new()),
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Async: context with stdin, env, and files (awk uses this).
pub async fn make_ctx_with_env_and_files(
    args: Vec<&str>,
    stdin: &str,
    env: HashMap<String, String>,
    files: Vec<(&str, &str)>,
) -> CommandContext {
    let fs = Arc::new(InMemoryFs::new());
    for (path, content) in files {
        fs.write_file(path, content.as_bytes()).await.unwrap();
    }
    CommandContext {
        args: args.into_iter().map(String::from).collect(),
        stdin: stdin.to_string(),
        cwd: "/".to_string(),
        env,
        fs,
        exec_fn: None,
        fetch_fn: None,
    }
}

/// Assert success: exit_code == 0 and stdout matches.
#[allow(dead_code)]
pub fn assert_success(result: &CommandResult, expected_stdout: &str) {
    assert_eq!(result.exit_code, 0, "expected exit_code 0, got {}", result.exit_code);
    assert_eq!(result.stdout, expected_stdout);
}

/// Assert failure: exit_code != 0.
#[allow(dead_code)]
pub fn assert_failure(result: &CommandResult) {
    assert_ne!(result.exit_code, 0, "expected non-zero exit_code");
}
