//! Control Flow Errors
//!
//! Error types used to implement shell control flow:
//! - break: Exit loops
//! - continue: Skip to next iteration
//! - return: Exit functions
//! - errexit: Exit on error (set -e)
//! - nounset: Error on unset variables (set -u)
//!
//! All control flow errors carry stdout/stderr to accumulate output
//! as they propagate through the execution stack.

use std::fmt;

/// Base trait for control flow errors that carry stdout/stderr.
pub trait ControlFlowError: std::error::Error {
    fn stdout(&self) -> &str;
    fn stderr(&self) -> &str;
    fn stdout_mut(&mut self) -> &mut String;
    fn stderr_mut(&mut self) -> &mut String;

    /// Prepend output from the current context before re-throwing.
    fn prepend_output(&mut self, stdout: &str, stderr: &str) {
        let new_stdout = format!("{}{}", stdout, self.stdout());
        let new_stderr = format!("{}{}", stderr, self.stderr());
        *self.stdout_mut() = new_stdout;
        *self.stderr_mut() = new_stderr;
    }
}

/// Implements Display, Error, ControlFlowError, and From<T> for InterpreterError
/// for each error type. All error types carry stdout/stderr fields.
macro_rules! impl_error {
    ($ty:ty, $variant:ident, $display:expr) => {
        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                #[allow(clippy::redundant_closure_call)]
                write!(f, "{}", ($display)(self))
            }
        }
        impl std::error::Error for $ty {}
        impl ControlFlowError for $ty {
            fn stdout(&self) -> &str { &self.stdout }
            fn stderr(&self) -> &str { &self.stderr }
            fn stdout_mut(&mut self) -> &mut String { &mut self.stdout }
            fn stderr_mut(&mut self) -> &mut String { &mut self.stderr }
        }
        impl From<$ty> for InterpreterError {
            fn from(e: $ty) -> Self { InterpreterError::$variant(e) }
        }
    };
}

// ---------------------------------------------------------------------------
// Error structs
// ---------------------------------------------------------------------------

/// Error thrown when break is called to exit loops.
#[derive(Debug, Clone)]
pub struct BreakError {
    pub levels: u32,
    pub stdout: String,
    pub stderr: String,
}

impl BreakError {
    pub fn new(levels: u32, stdout: String, stderr: String) -> Self {
        Self { levels, stdout, stderr }
    }
}

impl Default for BreakError {
    fn default() -> Self {
        Self { levels: 1, stdout: String::new(), stderr: String::new() }
    }
}

/// Error thrown when continue is called to skip to next iteration.
#[derive(Debug, Clone)]
pub struct ContinueError {
    pub levels: u32,
    pub stdout: String,
    pub stderr: String,
}

impl ContinueError {
    pub fn new(levels: u32, stdout: String, stderr: String) -> Self {
        Self { levels, stdout, stderr }
    }
}

impl Default for ContinueError {
    fn default() -> Self {
        Self { levels: 1, stdout: String::new(), stderr: String::new() }
    }
}

/// Error thrown when return is called to exit a function.
#[derive(Debug, Clone)]
pub struct ReturnError {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ReturnError {
    pub fn new(exit_code: i32, stdout: String, stderr: String) -> Self {
        Self { exit_code, stdout, stderr }
    }
}

impl Default for ReturnError {
    fn default() -> Self {
        Self { exit_code: 0, stdout: String::new(), stderr: String::new() }
    }
}

/// Error thrown when set -e (errexit) is enabled and a command fails.
#[derive(Debug, Clone)]
pub struct ErrexitError {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ErrexitError {
    pub fn new(exit_code: i32, stdout: String, stderr: String) -> Self {
        Self { exit_code, stdout, stderr }
    }
}

/// Error thrown when set -u (nounset) is enabled and an unset variable is referenced.
#[derive(Debug, Clone)]
pub struct NounsetError {
    pub var_name: String,
    pub stdout: String,
    pub stderr: String,
}

impl NounsetError {
    pub fn new(var_name: String, stdout: String) -> Self {
        let stderr = format!("bash: {}: unbound variable\n", var_name);
        Self { var_name, stdout, stderr }
    }
}

/// Error thrown when exit builtin is called to terminate the script.
#[derive(Debug, Clone)]
pub struct ExitError {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExitError {
    pub fn new(exit_code: i32, stdout: String, stderr: String) -> Self {
        Self { exit_code, stdout, stderr }
    }
}

/// Helper: if stderr is empty, generate a default "bash: {msg}\n" message.
fn default_stderr(stderr: String, msg: &str) -> String {
    if stderr.is_empty() { format!("bash: {}\n", msg) } else { stderr }
}

/// Error thrown for arithmetic expression errors.
#[derive(Debug, Clone)]
pub struct ArithmeticError {
    pub message: String,
    pub stdout: String,
    pub stderr: String,
    /// If true, this error should abort script execution.
    pub fatal: bool,
}

impl ArithmeticError {
    pub fn new(message: String, stdout: String, stderr: String, fatal: bool) -> Self {
        Self { message: message.clone(), stdout, stderr: default_stderr(stderr, &message), fatal }
    }

    pub fn simple(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self::new(msg, String::new(), String::new(), false)
    }
}

/// Error thrown for bad substitution errors (e.g., ${#var:1:3}).
#[derive(Debug, Clone)]
pub struct BadSubstitutionError {
    pub message: String,
    pub stdout: String,
    pub stderr: String,
}

impl BadSubstitutionError {
    pub fn new(message: String, stdout: String, stderr: String) -> Self {
        let stderr = if stderr.is_empty() {
            format!("bash: {}: bad substitution\n", message)
        } else {
            stderr
        };
        Self { message, stdout, stderr }
    }

    pub fn simple(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self::new(msg, String::new(), String::new())
    }
}

/// Error thrown when failglob is enabled and a glob pattern has no matches.
#[derive(Debug, Clone)]
pub struct GlobError {
    pub pattern: String,
    pub stdout: String,
    pub stderr: String,
}

impl GlobError {
    pub fn new(pattern: String, stdout: String, stderr: String) -> Self {
        let stderr = if stderr.is_empty() {
            format!("bash: no match: {}\n", pattern)
        } else {
            stderr
        };
        Self { pattern, stdout, stderr }
    }

    pub fn simple(pattern: impl Into<String>) -> Self {
        let pat = pattern.into();
        Self::new(pat, String::new(), String::new())
    }
}

/// Error thrown for invalid brace expansions.
#[derive(Debug, Clone)]
pub struct BraceExpansionError {
    pub message: String,
    pub stdout: String,
    pub stderr: String,
}

impl BraceExpansionError {
    pub fn new(message: String, stdout: String, stderr: String) -> Self {
        Self { message: message.clone(), stdout, stderr: default_stderr(stderr, &message) }
    }

    pub fn simple(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self::new(msg, String::new(), String::new())
    }
}

/// The type of execution limit that was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitType {
    Recursion,
    Commands,
    Iterations,
}

impl fmt::Display for LimitType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LimitType::Recursion => write!(f, "recursion"),
            LimitType::Commands => write!(f, "commands"),
            LimitType::Iterations => write!(f, "iterations"),
        }
    }
}

/// Error thrown when execution limits are exceeded.
/// Exit code 126 indicates a limit was exceeded.
#[derive(Debug, Clone)]
pub struct ExecutionLimitError {
    pub message: String,
    pub limit_type: LimitType,
    pub stdout: String,
    pub stderr: String,
}

impl ExecutionLimitError {
    pub const EXIT_CODE: i32 = 126;

    pub fn new(message: String, limit_type: LimitType, stdout: String, stderr: String) -> Self {
        Self { message: message.clone(), limit_type, stdout, stderr: default_stderr(stderr, &message) }
    }

    pub fn simple(message: impl Into<String>, limit_type: LimitType) -> Self {
        let msg = message.into();
        Self::new(msg, limit_type, String::new(), String::new())
    }
}

/// Error thrown when break/continue is called in a subshell spawned from a loop context.
#[derive(Debug, Clone)]
pub struct SubshellExitError {
    pub stdout: String,
    pub stderr: String,
}

impl SubshellExitError {
    pub fn new(stdout: String, stderr: String) -> Self {
        Self { stdout, stderr }
    }
}

impl Default for SubshellExitError {
    fn default() -> Self {
        Self { stdout: String::new(), stderr: String::new() }
    }
}

/// Error thrown when a POSIX special builtin fails in POSIX mode.
#[derive(Debug, Clone)]
pub struct PosixFatalError {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl PosixFatalError {
    pub fn new(exit_code: i32, stdout: String, stderr: String) -> Self {
        Self { exit_code, stdout, stderr }
    }
}

// ---------------------------------------------------------------------------
// Trait impls via macro (Display + Error + ControlFlowError + From)
// ---------------------------------------------------------------------------

impl_error!(BreakError,            Break,            |_: &BreakError| "break");
impl_error!(ContinueError,        Continue,         |_: &ContinueError| "continue");
impl_error!(ReturnError,          Return,           |_: &ReturnError| "return");
impl_error!(ErrexitError,         Errexit,          |e: &ErrexitError| format!("errexit: command exited with status {}", e.exit_code));
impl_error!(NounsetError,         Nounset,          |e: &NounsetError| format!("{}: unbound variable", e.var_name));
impl_error!(ExitError,            Exit,             |_: &ExitError| "exit");
impl_error!(ArithmeticError,      Arithmetic,       |e: &ArithmeticError| e.message.clone());
impl_error!(BadSubstitutionError, BadSubstitution,  |e: &BadSubstitutionError| e.message.clone());
impl_error!(GlobError,            Glob,             |e: &GlobError| format!("no match: {}", e.pattern));
impl_error!(BraceExpansionError,  BraceExpansion,   |e: &BraceExpansionError| e.message.clone());
impl_error!(ExecutionLimitError,  ExecutionLimit,    |e: &ExecutionLimitError| e.message.clone());
impl_error!(SubshellExitError,    SubshellExit,     |_: &SubshellExitError| "subshell exit");
impl_error!(PosixFatalError,      PosixFatal,       |_: &PosixFatalError| "posix fatal error");

// ---------------------------------------------------------------------------
// Unified error enum
// ---------------------------------------------------------------------------

/// Unified error enum for all interpreter errors.
#[derive(Debug, Clone)]
pub enum InterpreterError {
    Break(BreakError),
    Continue(ContinueError),
    Return(ReturnError),
    Errexit(ErrexitError),
    Nounset(NounsetError),
    Exit(ExitError),
    Arithmetic(ArithmeticError),
    BadSubstitution(BadSubstitutionError),
    Glob(GlobError),
    BraceExpansion(BraceExpansionError),
    ExecutionLimit(ExecutionLimitError),
    SubshellExit(SubshellExitError),
    PosixFatal(PosixFatalError),
}

impl fmt::Display for InterpreterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Break(e) => write!(f, "{}", e),
            Self::Continue(e) => write!(f, "{}", e),
            Self::Return(e) => write!(f, "{}", e),
            Self::Errexit(e) => write!(f, "{}", e),
            Self::Nounset(e) => write!(f, "{}", e),
            Self::Exit(e) => write!(f, "{}", e),
            Self::Arithmetic(e) => write!(f, "{}", e),
            Self::BadSubstitution(e) => write!(f, "{}", e),
            Self::Glob(e) => write!(f, "{}", e),
            Self::BraceExpansion(e) => write!(f, "{}", e),
            Self::ExecutionLimit(e) => write!(f, "{}", e),
            Self::SubshellExit(e) => write!(f, "{}", e),
            Self::PosixFatal(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for InterpreterError {}

/// Check if an error is a scope exit error (return, break, continue).
/// These need special handling vs errexit/nounset which terminate execution.
pub fn is_scope_exit_error(error: &InterpreterError) -> bool {
    matches!(
        error,
        InterpreterError::Break(_) | InterpreterError::Continue(_) | InterpreterError::Return(_)
    )
}
