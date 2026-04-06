//! return - Return from a function with an exit code

use super::break_cmd::BuiltinResult;
use crate::interpreter::errors::{InterpreterError, ReturnError};
use crate::interpreter::helpers::builtin_args::{parse_numeric_arg, wrap_exit_code};
use crate::interpreter::types::InterpreterState;

/// Handle the return builtin command.
///
/// # Arguments
/// * `state` - The interpreter state
/// * `args` - Command arguments
///
/// # Returns
/// Ok(BuiltinResult) for error cases, Err(InterpreterError) for control flow
pub fn handle_return(
    state: &InterpreterState,
    args: &[String],
) -> Result<BuiltinResult, InterpreterError> {
    // Check if we're in a function or sourced script
    if state.call_depth == 0 && state.source_depth == 0 {
        return Ok(BuiltinResult::failure(
            "bash: return: can only `return' from a function or sourced script\n",
            1,
        ));
    }

    let exit_code = match parse_numeric_arg("return", args) {
        Ok(None) => state.last_exit_code,
        Ok(Some(n)) => wrap_exit_code(n),
        Err(msg) => return Ok(BuiltinResult::failure(&msg, 2)),
    };

    Err(ReturnError::new(exit_code, String::new(), String::new()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> InterpreterState {
        InterpreterState::default()
    }

    #[test]
    fn test_return_outside_function() {
        let state = make_state();
        let result = handle_return(&state, &[]);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.exit_code, 1);
        assert!(r.stderr.contains("can only `return'"));
    }

    #[test]
    fn test_return_in_function() {
        let mut state = make_state();
        state.call_depth = 1;
        let result = handle_return(&state, &[]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Return(e) => assert_eq!(e.exit_code, 0),
            _ => panic!("Expected ReturnError"),
        }
    }

    #[test]
    fn test_return_with_exit_code() {
        let mut state = make_state();
        state.call_depth = 1;
        let result = handle_return(&state, &["42".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Return(e) => assert_eq!(e.exit_code, 42),
            _ => panic!("Expected ReturnError"),
        }
    }

    #[test]
    fn test_return_with_negative_exit_code() {
        let mut state = make_state();
        state.call_depth = 1;
        let result = handle_return(&state, &["-1".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Return(e) => assert_eq!(e.exit_code, 255),
            _ => panic!("Expected ReturnError"),
        }
    }

    #[test]
    fn test_return_in_sourced_script() {
        let mut state = make_state();
        state.source_depth = 1;
        let result = handle_return(&state, &["5".to_string()]);
        assert!(result.is_err());
        match result.unwrap_err() {
            InterpreterError::Return(e) => assert_eq!(e.exit_code, 5),
            _ => panic!("Expected ReturnError"),
        }
    }
}
