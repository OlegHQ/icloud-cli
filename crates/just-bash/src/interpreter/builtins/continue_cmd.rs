//! continue - Skip to next loop iteration builtin

use super::break_cmd::BuiltinResult;
use crate::interpreter::errors::{ContinueError, InterpreterError, SubshellExitError};
use crate::interpreter::helpers::loop_helpers::parse_loop_levels;
use crate::interpreter::types::InterpreterState;

/// Handle the continue builtin command.
///
/// # Arguments
/// * `state` - The interpreter state
/// * `args` - Command arguments
///
/// # Returns
/// Ok(BuiltinResult) for success, Err(InterpreterError) for control flow
pub fn handle_continue(
    state: &InterpreterState,
    args: &[String],
) -> Result<BuiltinResult, InterpreterError> {
    // Check if we're in a loop
    if state.loop_depth == 0 {
        // If we're in a subshell spawned from a loop context, exit the subshell
        if state.parent_has_loop_context.unwrap_or(false) {
            return Err(SubshellExitError::default().into());
        }
        // Otherwise, continue silently does nothing (returns 0)
        return Ok(BuiltinResult::ok());
    }

    let levels = parse_loop_levels("continue", args)?;
    Err(ContinueError::new(levels, String::new(), String::new()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> InterpreterState {
        InterpreterState::default()
    }

    #[test]
    fn test_continue_outside_loop() {
        let state = make_state();
        let result = handle_continue(&state, &[]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().exit_code, 0);
    }

    #[test]
    fn test_continue_in_loop() {
        let mut state = make_state();
        state.loop_depth = 1;
        let result = handle_continue(&state, &[]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Continue(e) => assert_eq!(e.levels, 1),
            _ => panic!("Expected ContinueError"),
        }
    }

    #[test]
    fn test_continue_with_levels() {
        let mut state = make_state();
        state.loop_depth = 3;
        let result = handle_continue(&state, &["2".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Continue(e) => assert_eq!(e.levels, 2),
            _ => panic!("Expected ContinueError"),
        }
    }
}
