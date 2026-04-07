//! Redirection Handling
//!
//! Handles output redirections:
//! - > : Write stdout to file
//! - >> : Append stdout to file
//! - 2> : Write stderr to file
//! - &> : Write both stdout and stderr to file
//! - >& : Redirect fd to another fd
//! - <<< : Here-string
//! - << / <<- : Here-document

use brush_parser::ast as bast;
use crate::interpreter::interpreter::FileSystem;
use crate::interpreter::types::{ExecResult, InterpreterState};
use std::collections::HashMap;

/// Pre-expanded redirect targets, keyed by index into the redirections array.
/// This allows us to expand redirect targets (including side effects) before
/// executing a function body, then apply the redirections after.
pub type ExpandedRedirectTargets = HashMap<usize, String>;

/// Result of checking an output redirect target
pub struct RedirectCheckResult {
    pub is_valid: bool,
    pub error: Option<String>,
}

/// Check if a redirect target is valid for output (not a directory, respects noclobber).
/// Returns an error message string if invalid, None if valid.
pub fn check_output_redirect_target(
    state: &InterpreterState,
    fs: &dyn FileSystem,
    file_path: &str,
    target: &str,
    check_noclobber: bool,
    is_clobber: bool,
) -> Option<String> {
    match fs.stat(file_path) {
        Ok(stat) => {
            if stat.is_dir {
                return Some(format!("bash: {}: Is a directory\n", target));
            }
            if check_noclobber && state.options.noclobber && !is_clobber && target != "/dev/null" {
                return Some(format!(
                    "bash: {}: cannot overwrite existing file\n",
                    target
                ));
            }
        }
        Err(_) => {
            // File doesn't exist, that's ok - we'll create it
        }
    }
    None
}

/// Determine the encoding to use for file I/O.
/// If all character codes are <= 255, use binary encoding (byte data).
/// Otherwise, use UTF-8 encoding (text with Unicode characters).
pub fn get_file_encoding(content: &str) -> &'static str {
    for ch in content.chars() {
        if ch as u32 > 255 {
            return "utf8";
        }
    }
    "binary"
}

/// Parse the content of a read-write file descriptor.
/// Format: __rw__:pathLength:path:position:content
/// Returns the parsed components, or None if format is invalid
pub fn parse_rw_fd_content(fd_content: &str) -> Option<RwFdContent> {
    if !fd_content.starts_with("__rw__:") {
        return None;
    }
    let after_prefix = &fd_content[7..];
    let first_colon_idx = after_prefix.find(':')?;
    let path_length: usize = after_prefix[..first_colon_idx].parse().ok()?;
    let path_start = first_colon_idx + 1;
    if path_start + path_length > after_prefix.len() {
        return None;
    }
    let path = after_prefix[path_start..path_start + path_length].to_string();
    let position_start = path_start + path_length + 1;
    if position_start >= after_prefix.len() {
        return None;
    }
    let remaining = &after_prefix[position_start..];
    let pos_colon_idx = remaining.find(':')?;
    let position: usize = remaining[..pos_colon_idx].parse().ok()?;
    let content = remaining[pos_colon_idx + 1..].to_string();
    Some(RwFdContent {
        path,
        position,
        content,
    })
}

/// Parsed content of a read-write file descriptor
#[derive(Debug, Clone)]
pub struct RwFdContent {
    pub path: String,
    pub position: usize,
    pub content: String,
}

/// Allocate the next available file descriptor (starting at 10).
/// Returns the allocated FD number.
pub fn allocate_fd(state: &mut InterpreterState) -> i32 {
    let fd = state.next_fd.unwrap_or(10);
    state.next_fd = Some(fd + 1);
    fd
}

/// Result of pre-expanding redirect targets
pub struct PreExpandResult {
    pub targets: ExpandedRedirectTargets,
    pub error: Option<String>,
}

/// Helper: extract the word value from an IoRedirect (for file targets).
/// Returns None for here-documents.
fn get_redirect_word(redir: &bast::IoRedirect) -> Option<&bast::Word> {
    match redir {
        bast::IoRedirect::File(_, _, bast::IoFileRedirectTarget::Filename(w)) => Some(w),
        bast::IoRedirect::File(_, _, bast::IoFileRedirectTarget::Duplicate(w)) => Some(w),
        bast::IoRedirect::HereString(_, w) => Some(w),
        bast::IoRedirect::OutputAndError(w, _) => Some(w),
        _ => None,
    }
}

/// Pre-expand redirect targets for function definitions.
/// This is needed because redirections on function definitions are evaluated
/// each time the function is called, and any side effects (like $((i++)))
/// must occur BEFORE the function body executes.
pub fn pre_expand_redirect_targets(
    state: &mut InterpreterState,
    redirections: &[bast::IoRedirect],
    expand_word_fn: impl Fn(&mut InterpreterState, &bast::Word) -> String,
) -> PreExpandResult {
    let mut targets = ExpandedRedirectTargets::new();

    for (i, redir) in redirections.iter().enumerate() {
        // Skip heredocs
        if matches!(redir, bast::IoRedirect::HereDocument(..)) {
            continue;
        }

        if let Some(word) = get_redirect_word(redir) {
            let target = expand_word_fn(state, word);
            targets.insert(i, target);
        }
    }

    PreExpandResult {
        targets,
        error: None,
    }
}

// NOTE: brush-parser's IoRedirect does not support the `{varname}>file` syntax
// for dynamic FD allocation (fd_variable). This is a bash extension that would
// need custom handling. The process_fd_variable_redirections function is retained
// but adapted to work without that feature. Callers that need fd_variable support
// should handle it at a higher level.

/// Process FD variable redirections ({varname}>file syntax).
/// NOTE: brush-parser does not encode fd_variable in IoRedirect. This function
/// is kept as a no-op placeholder for forward compatibility; callers needing
/// fd_variable support should layer it on top.
pub fn process_fd_variable_redirections(
    _state: &mut InterpreterState,
    _redirections: &[bast::IoRedirect],
    _fs: &dyn FileSystem,
    _expand_word_fn: impl Fn(&mut InterpreterState, &bast::Word) -> String,
) -> Option<ExecResult> {
    // fd_variable is not represented in brush-parser's IoRedirect.
    // Dynamic FD allocation ({varname}>file) is not supported in this path.
    None
}

/// Pre-open (truncate) output redirect files before command execution.
/// This is needed for compound commands (subshell, for, case, [[) where
/// bash opens/truncates the redirect file BEFORE evaluating any words in
/// the command body (including command substitutions).
///
/// Example: `(echo $(cat FILE)) > FILE`
/// - Bash first truncates FILE (making it empty)
/// - Then executes the subshell, where `cat FILE` returns empty string
///
/// Returns an error result if there's an issue (like directory or noclobber),
/// or None if pre-opening succeeded.
pub fn pre_open_output_redirects(
    state: &mut InterpreterState,
    redirections: &[bast::IoRedirect],
    fs: &dyn FileSystem,
    expand_word_fn: impl Fn(&mut InterpreterState, &bast::Word) -> String,
) -> Option<ExecResult> {
    for redir in redirections {
        match redir {
            bast::IoRedirect::HereDocument(..) | bast::IoRedirect::HereString(..) => {
                continue;
            }

            bast::IoRedirect::File(fd, kind, target_node) => {
                // Only handle output truncation redirects (>, >|)
                // Append (>>) doesn't need pre-truncation
                // >& needs special handling - it's a file redirect only if word is not a number
                let is_duplicate_output = matches!(kind, bast::IoFileRedirectKind::DuplicateOutput);
                if !matches!(
                    kind,
                    bast::IoFileRedirectKind::Write | bast::IoFileRedirectKind::Clobber
                ) && !is_duplicate_output
                {
                    continue;
                }

                let target = match target_node {
                    bast::IoFileRedirectTarget::Filename(w) => expand_word_fn(state, w),
                    bast::IoFileRedirectTarget::Duplicate(w) => expand_word_fn(state, w),
                    bast::IoFileRedirectTarget::Fd(_) => continue,
                    bast::IoFileRedirectTarget::ProcessSubstitution(..) => continue,
                };

                // For >&, check if it's an FD redirect (number or -)
                if is_duplicate_output {
                    if target == "-" || target.parse::<i32>().is_ok() || fd.is_some() {
                        continue;
                    }
                }

                let file_path = fs.resolve_path(&state.cwd, &target);
                let is_clobber = matches!(kind, bast::IoFileRedirectKind::Clobber);

                // Check if target is a directory or noclobber prevents overwrite
                match fs.stat(&file_path) {
                    Ok(stat) => {
                        if stat.is_dir {
                            return Some(ExecResult::new(
                                String::new(),
                                format!("bash: {}: Is a directory\n", target),
                                1,
                            ));
                        }
                        if state.options.noclobber && !is_clobber && target != "/dev/null" {
                            return Some(ExecResult::new(
                                String::new(),
                                format!("bash: {}: cannot overwrite existing file\n", target),
                                1,
                            ));
                        }
                    }
                    Err(_) => {
                        // File doesn't exist, that's ok - we'll create it
                    }
                }

                // Pre-truncate the file (create empty file)
                // Skip special device files that don't need pre-truncation
                if target != "/dev/null"
                    && target != "/dev/stdout"
                    && target != "/dev/stderr"
                    && target != "/dev/full"
                {
                    let _ = fs.write_file(&file_path, "");
                }

                // /dev/full always returns ENOSPC when written to
                if target == "/dev/full" {
                    return Some(ExecResult::new(
                        String::new(),
                        "bash: /dev/full: No space left on device\n".to_string(),
                        1,
                    ));
                }
            }

            bast::IoRedirect::OutputAndError(word, append) => {
                // &> needs pre-truncation, &>> does not
                if *append {
                    continue;
                }

                let target = expand_word_fn(state, word);
                let file_path = fs.resolve_path(&state.cwd, &target);

                // Check if target is a directory or noclobber prevents overwrite
                match fs.stat(&file_path) {
                    Ok(stat) => {
                        if stat.is_dir {
                            return Some(ExecResult::new(
                                String::new(),
                                format!("bash: {}: Is a directory\n", target),
                                1,
                            ));
                        }
                        if state.options.noclobber && target != "/dev/null" {
                            return Some(ExecResult::new(
                                String::new(),
                                format!("bash: {}: cannot overwrite existing file\n", target),
                                1,
                            ));
                        }
                    }
                    Err(_) => {}
                }

                if target != "/dev/null"
                    && target != "/dev/stdout"
                    && target != "/dev/stderr"
                    && target != "/dev/full"
                {
                    let _ = fs.write_file(&file_path, "");
                }

                if target == "/dev/full" {
                    return Some(ExecResult::new(
                        String::new(),
                        "bash: /dev/full: No space left on device\n".to_string(),
                        1,
                    ));
                }
            }
        }
    }

    None // Success - no error
}

/// Apply redirections to an execution result.
/// This handles the actual redirection of stdout/stderr to files or other FDs.
pub fn apply_redirections(
    state: &mut InterpreterState,
    result: ExecResult,
    redirections: &[bast::IoRedirect],
    pre_expanded_targets: Option<&ExpandedRedirectTargets>,
    fs: &dyn FileSystem,
    expand_word_fn: impl Fn(&mut InterpreterState, &bast::Word) -> String,
) -> ExecResult {
    let mut stdout = result.stdout;
    let mut stderr = result.stderr;
    let mut exit_code = result.exit_code;

    for (i, redir) in redirections.iter().enumerate() {
        match redir {
            // Skip heredocs
            bast::IoRedirect::HereDocument(..) => {
                continue;
            }

            bast::IoRedirect::HereString(..) => {
                // Here-strings are handled at input stage, not output
                continue;
            }

            bast::IoRedirect::File(fd_opt, kind, target_node) => {
                // Extract the target string
                let target = match target_node {
                    bast::IoFileRedirectTarget::Filename(w) => {
                        if let Some(targets) = pre_expanded_targets {
                            targets
                                .get(&i)
                                .cloned()
                                .unwrap_or_else(|| expand_word_fn(state, w))
                        } else {
                            expand_word_fn(state, w)
                        }
                    }
                    bast::IoFileRedirectTarget::Duplicate(w) => {
                        if let Some(targets) = pre_expanded_targets {
                            targets
                                .get(&i)
                                .cloned()
                                .unwrap_or_else(|| expand_word_fn(state, w))
                        } else {
                            expand_word_fn(state, w)
                        }
                    }
                    bast::IoFileRedirectTarget::Fd(fd_num) => fd_num.to_string(),
                    bast::IoFileRedirectTarget::ProcessSubstitution(..) => continue,
                };

                match kind {
                    bast::IoFileRedirectKind::Write | bast::IoFileRedirectKind::Clobber => {
                        let fd = fd_opt.unwrap_or(1);
                        let is_clobber = matches!(kind, bast::IoFileRedirectKind::Clobber);

                        if fd == 1 {
                            if target == "/dev/stdout" {
                                // No-op for stdout
                            } else if target == "/dev/stderr" {
                                stderr.push_str(&stdout);
                                stdout.clear();
                            } else if target == "/dev/full" {
                                stderr.push_str(
                                    "bash: echo: write error: No space left on device\n",
                                );
                                exit_code = 1;
                                stdout.clear();
                            } else if target == "/dev/null" {
                                stdout.clear();
                            } else {
                                let file_path = fs.resolve_path(&state.cwd, &target);
                                if let Some(err) = check_output_redirect_target(
                                    state, fs, &file_path, &target, true, is_clobber,
                                ) {
                                    stderr.push_str(&err);
                                    exit_code = 1;
                                    stdout.clear();
                                } else {
                                    let _ = fs.write_file(&file_path, &stdout);
                                    stdout.clear();
                                }
                            }
                        } else if fd == 2 {
                            if target == "/dev/stderr" {
                                // No-op for stderr
                            } else if target == "/dev/stdout" {
                                stdout.push_str(&stderr);
                                stderr.clear();
                            } else if target == "/dev/full" {
                                stderr.push_str(
                                    "bash: echo: write error: No space left on device\n",
                                );
                                exit_code = 1;
                            } else if target == "/dev/null" {
                                stderr.clear();
                            } else {
                                let file_path = fs.resolve_path(&state.cwd, &target);
                                if let Some(err) = check_output_redirect_target(
                                    state, fs, &file_path, &target, true, is_clobber,
                                ) {
                                    stderr.push_str(&err);
                                    exit_code = 1;
                                } else {
                                    let _ = fs.write_file(&file_path, &stderr);
                                    stderr.clear();
                                }
                            }
                        }
                    }

                    bast::IoFileRedirectKind::Append => {
                        let fd = fd_opt.unwrap_or(1);

                        if fd == 1 {
                            if target == "/dev/stdout" {
                                // No-op
                            } else if target == "/dev/stderr" {
                                stderr.push_str(&stdout);
                                stdout.clear();
                            } else if target == "/dev/full" {
                                stderr.push_str(
                                    "bash: echo: write error: No space left on device\n",
                                );
                                exit_code = 1;
                                stdout.clear();
                            } else {
                                let file_path = fs.resolve_path(&state.cwd, &target);
                                if let Some(err) = check_output_redirect_target(
                                    state, fs, &file_path, &target, false, false,
                                ) {
                                    stderr.push_str(&err);
                                    exit_code = 1;
                                    stdout.clear();
                                } else {
                                    let _ = fs.append_file(&file_path, &stdout);
                                    stdout.clear();
                                }
                            }
                        } else if fd == 2 {
                            if target == "/dev/stderr" {
                                // No-op
                            } else if target == "/dev/stdout" {
                                stdout.push_str(&stderr);
                                stderr.clear();
                            } else if target == "/dev/full" {
                                stderr.push_str(
                                    "bash: echo: write error: No space left on device\n",
                                );
                                exit_code = 1;
                            } else {
                                let file_path = fs.resolve_path(&state.cwd, &target);
                                if let Some(err) = check_output_redirect_target(
                                    state, fs, &file_path, &target, false, false,
                                ) {
                                    stderr.push_str(&err);
                                    exit_code = 1;
                                } else {
                                    let _ = fs.append_file(&file_path, &stderr);
                                    stderr.clear();
                                }
                            }
                        }
                    }

                    bast::IoFileRedirectKind::DuplicateOutput
                    | bast::IoFileRedirectKind::DuplicateInput => {
                        let fd = fd_opt.unwrap_or(1);

                        // Handle close operation
                        if target == "-" {
                            continue;
                        }

                        // Handle FD move operation: N>&M-
                        if target.ends_with('-') {
                            let source_fd_str = &target[..target.len() - 1];
                            if let Ok(source_fd) = source_fd_str.parse::<i32>() {
                                // Duplicate: copy content from source to target FD
                                if source_fd == 1 && fd == 2 {
                                    stderr.push_str(&stdout);
                                } else if source_fd == 2 && fd == 1 {
                                    stdout.push_str(&stderr);
                                }
                                // Close the source FD
                                if source_fd == 1 {
                                    stdout.clear();
                                } else if source_fd == 2 {
                                    stderr.clear();
                                }
                                // Mark the move in persistent FDs
                                if let Some(ref mut fds) = state.file_descriptors {
                                    if let Some(content) = fds.get(&source_fd).cloned() {
                                        fds.insert(fd, content);
                                    }
                                    fds.remove(&source_fd);
                                }
                                continue;
                            }
                        }

                        // >&2, 1>&2: redirect stdout to stderr
                        if target == "2" || target == "&2" {
                            if fd == 1 {
                                stderr.push_str(&stdout);
                                stdout.clear();
                            }
                        }
                        // 2>&1: redirect stderr to stdout
                        else if target == "1" || target == "&1" {
                            if fd == 2 {
                                stdout.push_str(&stderr);
                                stderr.clear();
                            }
                        }
                        // Handle writing to a user-allocated FD
                        else if let Ok(target_fd) = target.parse::<i32>() {
                            if let Some(ref fds) = state.file_descriptors {
                                if let Some(fd_info) = fds.get(&target_fd) {
                                    if fd_info.starts_with("__file__:") {
                                        let file_path = &fd_info[9..];
                                        if fd == 1 {
                                            let _ = fs.append_file(file_path, &stdout);
                                            stdout.clear();
                                        } else if fd == 2 {
                                            let _ = fs.append_file(file_path, &stderr);
                                            stderr.clear();
                                        }
                                    }
                                } else if target_fd >= 3 {
                                    // Bad file descriptor
                                    stderr.push_str(&format!(
                                        "bash: {}: Bad file descriptor\n",
                                        target_fd
                                    ));
                                    exit_code = 1;
                                    stdout.clear();
                                }
                            }
                        }
                    }

                    bast::IoFileRedirectKind::Read
                    | bast::IoFileRedirectKind::ReadAndWrite => {
                        // Input redirections - handled at input stage
                    }
                }
            }

            bast::IoRedirect::OutputAndError(word, append) => {
                let target = if let Some(targets) = pre_expanded_targets {
                    targets
                        .get(&i)
                        .cloned()
                        .unwrap_or_else(|| expand_word_fn(state, word))
                } else {
                    expand_word_fn(state, word)
                };

                if *append {
                    // &>>
                    if target == "/dev/full" {
                        stderr =
                            "bash: echo: write error: No space left on device\n".to_string();
                        exit_code = 1;
                        stdout.clear();
                    } else {
                        let file_path = fs.resolve_path(&state.cwd, &target);
                        let combined = format!("{}{}", stdout, stderr);
                        let _ = fs.append_file(&file_path, &combined);
                        stdout.clear();
                        stderr.clear();
                    }
                } else {
                    // &>
                    if target == "/dev/full" {
                        stderr =
                            "bash: echo: write error: No space left on device\n".to_string();
                        exit_code = 1;
                        stdout.clear();
                    } else {
                        let file_path = fs.resolve_path(&state.cwd, &target);
                        let combined = format!("{}{}", stdout, stderr);
                        let _ = fs.write_file(&file_path, &combined);
                        stdout.clear();
                        stderr.clear();
                    }
                }
            }
        }
    }

    // Apply persistent FD redirections (from exec)
    if let Some(ref fds) = state.file_descriptors {
        if let Some(fd1_info) = fds.get(&1) {
            if fd1_info == "__dupout__:2" {
                stderr.push_str(&stdout);
                stdout.clear();
            } else if fd1_info.starts_with("__file__:") {
                let file_path = &fd1_info[9..];
                let _ = fs.append_file(file_path, &stdout);
                stdout.clear();
            }
        }

        if let Some(fd2_info) = fds.get(&2) {
            if fd2_info == "__dupout__:1" {
                stdout.push_str(&stderr);
                stderr.clear();
            } else if fd2_info.starts_with("__file__:") {
                let file_path = &fd2_info[9..];
                let _ = fs.append_file(file_path, &stderr);
                stderr.clear();
            }
        }
    }

    ExecResult::new(stdout, stderr, exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_file_encoding_binary() {
        assert_eq!(get_file_encoding("hello world"), "binary");
        assert_eq!(get_file_encoding("abc123"), "binary");
    }

    #[test]
    fn test_get_file_encoding_utf8() {
        assert_eq!(get_file_encoding("hello 世界"), "utf8");
        assert_eq!(get_file_encoding("日本語"), "utf8");
    }

    #[test]
    fn test_parse_rw_fd_content_valid() {
        let content = "__rw__:4:/tmp:10:hello";
        let parsed = parse_rw_fd_content(content).unwrap();
        assert_eq!(parsed.path, "/tmp");
        assert_eq!(parsed.position, 10);
        assert_eq!(parsed.content, "hello");
    }

    #[test]
    fn test_parse_rw_fd_content_invalid() {
        assert!(parse_rw_fd_content("invalid").is_none());
        assert!(parse_rw_fd_content("__rw__:").is_none());
        assert!(parse_rw_fd_content("__rw__:abc:").is_none());
    }

    #[test]
    fn test_allocate_fd() {
        let mut state = InterpreterState::default();
        assert_eq!(allocate_fd(&mut state), 10);
        assert_eq!(allocate_fd(&mut state), 11);
        assert_eq!(allocate_fd(&mut state), 12);
    }
}
