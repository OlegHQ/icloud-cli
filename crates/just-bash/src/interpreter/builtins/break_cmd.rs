//! break - Exit from loops builtin

use crate::interpreter::errors::{BreakError, InterpreterError, SubshellExitError};
use crate::interpreter::helpers::loop_helpers::parse_loop_levels;
use crate::interpreter::types::InterpreterState;

/// Result type for builtin commands.
#[derive(Debug, Clone)]
pub struct BuiltinResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl BuiltinResult {
    pub fn ok() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        }
    }

    pub fn failure(stderr: &str, exit_code: i32) -> Self {
        Self {
            stdout: String::new(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }
}

/// Handle the break builtin command.
///
/// # Arguments
/// * `state` - The interpreter state
/// * `args` - Command arguments
///
/// # Returns
/// Ok(BuiltinResult) for success, Err(InterpreterError) for control flow
pub fn handle_break(
    state: &InterpreterState,
    args: &[String],
) -> Result<BuiltinResult, InterpreterError> {
    // Check if we're in a loop
    if state.loop_depth == 0 {
        // If we're in a subshell spawned from a loop context, exit the subshell
        if state.parent_has_loop_context.unwrap_or(false) {
            return Err(SubshellExitError::default().into());
        }
        // Otherwise, break silently does nothing (returns 0)
        return Ok(BuiltinResult::ok());
    }

    let levels = parse_loop_levels("break", args)?;
    Err(BreakError::new(levels, String::new(), String::new()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> InterpreterState {
        InterpreterState::default()
    }

    #[test]
    fn test_break_outside_loop() {
        let state = make_state();
        let result = handle_break(&state, &[]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().exit_code, 0);
    }

    #[test]
    fn test_break_in_loop() {
        let mut state = make_state();
        state.loop_depth = 1;
        let result = handle_break(&state, &[]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Break(e) => assert_eq!(e.levels, 1),
            _ => panic!("Expected BreakError"),
        }
    }

    #[test]
    fn test_break_with_levels() {
        let mut state = make_state();
        state.loop_depth = 3;
        let result = handle_break(&state, &["2".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Break(e) => assert_eq!(e.levels, 2),
            _ => panic!("Expected BreakError"),
        }
    }

    #[test]
    fn test_break_invalid_argument() {
        let mut state = make_state();
        state.loop_depth = 1;
        let result = handle_break(&state, &["abc".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Exit(e) => {
                assert_eq!(e.exit_code, 128);
                assert!(e.stderr.contains("numeric argument required"));
            }
            _ => panic!("Expected ExitError"),
        }
    }

    #[test]
    fn test_break_too_many_args() {
        let mut state = make_state();
        state.loop_depth = 1;
        let result = handle_break(&state, &["1".to_string(), "2".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Exit(e) => {
                assert_eq!(e.exit_code, 1);
                assert!(e.stderr.contains("too many arguments"));
            }
            _ => panic!("Expected ExitError"),
        }
    }
}
