//! Array Word Expansion Handlers
//!
//! Handles complex array expansion cases in word expansion:
//! - "${arr[@]}" and "${arr[*]}" - array element expansion
//! - "${arr[@]:-default}" - array with defaults
//! - "${arr[@]:offset:length}" - array slicing
//! - "${arr[@]/pattern/replacement}" - pattern replacement
//! - "${arr[@]#pattern}" - pattern removal
//! - "${arr[@]@op}" - transform operations

use brush_parser::word::{
    Parameter, ParameterExpr, WordPiece, WordPieceWithSource,
};
use crate::interpreter::expansion::get_array_elements;
use crate::interpreter::helpers::{get_nameref_target, is_nameref};
use crate::interpreter::InterpreterState;
use regex_lite::Regex;

/// Result type for array expansion handlers.
/// `None` means the handler doesn't apply to this case.
#[derive(Debug, Clone)]
pub struct ArrayExpansionResult {
    pub values: Vec<String>,
    pub quoted: bool,
}

/// Handle simple "${arr[@]}" expansion without operations (brush-parser types).
/// Returns each array element as a separate word.
pub fn handle_simple_array_expansion_bp(
    state: &InterpreterState,
    word_pieces: &[WordPieceWithSource],
) -> Option<ArrayExpansionResult> {
    if word_pieces.len() != 1 {
        return None;
    }

    // Must be a double-quoted sequence with a single parameter expansion
    let dq_pieces = match &word_pieces[0].piece {
        WordPiece::DoubleQuotedSequence(inner) => inner,
        _ => return None,
    };

    if dq_pieces.len() != 1 {
        return None;
    }

    let param_expr = match &dq_pieces[0].piece {
        WordPiece::ParameterExpansion(expr) => expr,
        _ => return None,
    };

    // Must be a simple Parameter (no operations like Substring, Transform, etc.)
    let (parameter, _indirect) = match param_expr {
        ParameterExpr::Parameter { parameter, indirect } => (parameter, indirect),
        _ => return None,
    };

    // Must be NamedWithAllIndices with concatenate=false (i.e., arr[@])
    let array_name = match parameter {
        Parameter::NamedWithAllIndices { name, concatenate } if !concatenate => name,
        _ => return None,
    };

    // Special case: if arrayName is a nameref pointing to array[@],
    // ${ref[@]} doesn't do double indirection - it returns empty
    if is_nameref(state, array_name) {
        if let Some(target) = get_nameref_target(state, &state.env, array_name) {
            if target.ends_with("[@]") || target.ends_with("[*]") {
                return Some(ArrayExpansionResult {
                    values: vec![],
                    quoted: true,
                });
            }
        }
    }

    let elements = get_array_elements(state, array_name);
    if !elements.is_empty() {
        return Some(ArrayExpansionResult {
            values: elements.into_iter().map(|(_, v)| v).collect(),
            quoted: true,
        });
    }

    // No array elements - check for scalar variable
    if let Some(scalar_value) = state.env.get(array_name.as_str()) {
        return Some(ArrayExpansionResult {
            values: vec![scalar_value.clone()],
            quoted: true,
        });
    }

    // Variable is unset - return empty
    Some(ArrayExpansionResult {
        values: vec![],
        quoted: true,
    })
}

/// Handle namerefs pointing to array[@] - "${ref}" where ref='arr[@]' (brush-parser types).
/// When a nameref points to array[@], expanding "$ref" should produce multiple words.
pub fn handle_nameref_array_expansion_bp(
    state: &InterpreterState,
    word_pieces: &[WordPieceWithSource],
) -> Option<ArrayExpansionResult> {
    if word_pieces.len() != 1 {
        return None;
    }

    let dq_pieces = match &word_pieces[0].piece {
        WordPiece::DoubleQuotedSequence(inner) => inner,
        _ => return None,
    };

    if dq_pieces.len() != 1 {
        return None;
    }

    let param_expr = match &dq_pieces[0].piece {
        WordPiece::ParameterExpansion(expr) => expr,
        _ => return None,
    };

    // Must be a simple Parameter, no operations
    let var_name = match param_expr {
        ParameterExpr::Parameter { parameter: Parameter::Named(name), indirect: false } => name,
        _ => return None,
    };

    // Check if it's a nameref
    if !is_nameref(state, var_name) {
        return None;
    }

    let target = get_nameref_target(state, &state.env, var_name)?;

    // Check if resolved target is array[@]
    let target_array_re = Regex::new(r"^([a-zA-Z_][a-zA-Z0-9_]*)\[(@)\]$").unwrap();
    let caps = target_array_re.captures(&target)?;
    let array_name = caps.get(1)?.as_str();

    let elements = get_array_elements(state, array_name);
    if !elements.is_empty() {
        return Some(ArrayExpansionResult {
            values: elements.into_iter().map(|(_, v)| v).collect(),
            quoted: true,
        });
    }

    // No array elements - check for scalar variable
    if let Some(scalar_value) = state.env.get(array_name) {
        return Some(ArrayExpansionResult {
            values: vec![scalar_value.clone()],
            quoted: true,
        });
    }

    // Variable is unset - return empty
    Some(ArrayExpansionResult {
        values: vec![],
        quoted: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use brush_parser::word::{SpecialParameter, WordPieceWithSource};
    use std::collections::HashMap;

    fn make_state() -> InterpreterState {
        InterpreterState {
            env: HashMap::new(),
            ..Default::default()
        }
    }

    fn make_piece_with_source(piece: WordPiece) -> WordPieceWithSource {
        WordPieceWithSource {
            piece,
            start_index: 0,
            end_index: 0,
        }
    }

    #[test]
    fn test_handle_simple_array_expansion_bp_empty() {
        let state = make_state();
        // Test with non-matching pattern (a plain text piece, not a double-quoted array expansion)
        let pieces = vec![make_piece_with_source(WordPiece::Text("foo".to_string()))];
        assert!(handle_simple_array_expansion_bp(&state, &pieces).is_none());
    }

    #[test]
    fn test_handle_simple_array_expansion_bp_with_array() {
        let mut state = make_state();
        state.env.insert("arr_0".to_string(), "first".to_string());
        state.env.insert("arr_1".to_string(), "second".to_string());
        state.env.insert("arr_2".to_string(), "third".to_string());

        // Create "${arr[@]}" word pieces
        let pieces = vec![make_piece_with_source(WordPiece::DoubleQuotedSequence(vec![
            make_piece_with_source(WordPiece::ParameterExpansion(
                ParameterExpr::Parameter {
                    parameter: Parameter::NamedWithAllIndices {
                        name: "arr".to_string(),
                        concatenate: false,
                    },
                    indirect: false,
                },
            )),
        ]))];

        let result = handle_simple_array_expansion_bp(&state, &pieces);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.values, vec!["first", "second", "third"]);
        assert!(result.quoted);
    }

    #[test]
    fn test_handle_simple_array_expansion_bp_scalar() {
        let mut state = make_state();
        state.env.insert("s".to_string(), "scalar".to_string());

        // Create "${s[@]}" word pieces
        let pieces = vec![make_piece_with_source(WordPiece::DoubleQuotedSequence(vec![
            make_piece_with_source(WordPiece::ParameterExpansion(
                ParameterExpr::Parameter {
                    parameter: Parameter::NamedWithAllIndices {
                        name: "s".to_string(),
                        concatenate: false,
                    },
                    indirect: false,
                },
            )),
        ]))];

        let result = handle_simple_array_expansion_bp(&state, &pieces);
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.values, vec!["scalar"]);
    }
}
