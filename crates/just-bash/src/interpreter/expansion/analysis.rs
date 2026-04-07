//! Word Analysis
//!
//! Functions for analyzing word pieces to determine what types of expansions are present.

use brush_parser::word::{ParameterExpr, WordPiece, WordPieceWithSource};

/// Check if a glob pattern string contains variable references ($var or ${var})
/// This is used to detect when IFS splitting should apply to expanded glob patterns.
pub fn glob_pattern_has_var_ref(pattern: &str) -> bool {
    // Look for $varname or ${...} patterns
    // Skip escaped $ (e.g., \$)
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2; // Skip next character
            continue;
        }
        if chars[i] == '$' {
            if let Some(&next) = chars.get(i + 1) {
                // Check for ${...} or $varname
                if next == '{' || next.is_ascii_alphabetic() || next == '_' {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Check if a parameter expression has a default/alternative/error value.
/// In brush-parser ParameterExpr, the value is stored as an Option<String>
/// in the variant directly.
fn has_quoted_default_value(expr: &ParameterExpr) -> bool {
    match expr {
        ParameterExpr::UseDefaultValues { default_value: Some(v), .. }
        | ParameterExpr::AssignDefaultValues { default_value: Some(v), .. }
        | ParameterExpr::UseAlternativeValue { alternative_value: Some(v), .. } => {
            // In brush-parser, the default value is a raw string that may contain quote markers.
            // Check if it contains any quoting.
            v.contains('"') || v.contains('\'')
        }
        ParameterExpr::IndicateErrorIfNullOrUnset { error_message: Some(v), .. } => {
            v.contains('"') || v.contains('\'')
        }
        _ => false,
    }
}

/// Check if a parameter expansion's operation value is entirely quoted.
/// In brush-parser, default/alternative values are raw strings, so we check
/// if the entire value is wrapped in quotes.
///
/// For word splitting purposes:
/// - ${v:-"AxBxC"} - entirely quoted, should NOT be split
/// - ${v:-x"AxBxC"x} - mixed quoted/unquoted, SHOULD be split
/// - ${v:-AxBxC} - entirely unquoted, SHOULD be split
pub fn is_operation_word_entirely_quoted(expr: &ParameterExpr) -> bool {
    let value = match expr {
        ParameterExpr::UseDefaultValues { default_value: Some(v), .. }
        | ParameterExpr::AssignDefaultValues { default_value: Some(v), .. }
        | ParameterExpr::UseAlternativeValue { alternative_value: Some(v), .. } => v,
        ParameterExpr::IndicateErrorIfNullOrUnset { error_message: Some(v), .. } => v,
        _ => return false,
    };

    if value.is_empty() {
        return false;
    }

    // Check if the entire value is wrapped in double quotes or single quotes
    (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
}

/// Result of analyzing word pieces
#[derive(Debug, Clone, Default)]
pub struct WordPartsAnalysis {
    pub has_quoted: bool,
    pub has_command_sub: bool,
    pub has_array_var: bool,
    pub has_array_at_expansion: bool,
    pub has_param_expansion: bool,
    pub has_var_name_prefix_expansion: bool,
    pub has_indirection: bool,
}

/// Check if a ParameterExpr is a pattern removal or replacement operation.
fn is_pattern_operation(expr: &ParameterExpr) -> bool {
    matches!(
        expr,
        ParameterExpr::RemoveSmallestSuffixPattern { .. }
            | ParameterExpr::RemoveLargestSuffixPattern { .. }
            | ParameterExpr::RemoveSmallestPrefixPattern { .. }
            | ParameterExpr::RemoveLargestPrefixPattern { .. }
            | ParameterExpr::ReplaceSubstring { .. }
    )
}

/// Check if the parameter refers to an array[@] or array[*].
fn is_array_all_indices_param(param: &brush_parser::word::Parameter) -> bool {
    matches!(param, brush_parser::word::Parameter::NamedWithAllIndices { .. })
}

/// Check if the parameter refers to @ or * special parameters.
fn is_positional_all_param(param: &brush_parser::word::Parameter) -> bool {
    matches!(
        param,
        brush_parser::word::Parameter::Special(
            brush_parser::word::SpecialParameter::AllPositionalParameters { .. }
        )
    )
}

/// Analyze a single ParameterExpr for expansion properties
fn analyze_parameter_expr(expr: &ParameterExpr, result: &mut WordPartsAnalysis) {
    match expr {
        ParameterExpr::VariableNames { .. } => {
            result.has_var_name_prefix_expansion = true;
        }
        ParameterExpr::MemberKeys { .. } => {
            result.has_var_name_prefix_expansion = true;
        }
        ParameterExpr::Parameter { parameter, indirect, .. } => {
            if *indirect {
                result.has_indirection = true;
            }
            if is_array_all_indices_param(parameter) {
                result.has_array_at_expansion = true;
            }
        }
        // For pattern operations on array[@], set has_array_at_expansion
        other => {
            // Check indirect flag
            let indirect = match other {
                ParameterExpr::Parameter { indirect, .. }
                | ParameterExpr::UseDefaultValues { indirect, .. }
                | ParameterExpr::AssignDefaultValues { indirect, .. }
                | ParameterExpr::IndicateErrorIfNullOrUnset { indirect, .. }
                | ParameterExpr::UseAlternativeValue { indirect, .. }
                | ParameterExpr::ParameterLength { indirect, .. }
                | ParameterExpr::RemoveSmallestSuffixPattern { indirect, .. }
                | ParameterExpr::RemoveLargestSuffixPattern { indirect, .. }
                | ParameterExpr::RemoveSmallestPrefixPattern { indirect, .. }
                | ParameterExpr::RemoveLargestPrefixPattern { indirect, .. }
                | ParameterExpr::Substring { indirect, .. }
                | ParameterExpr::Transform { indirect, .. }
                | ParameterExpr::UppercaseFirstChar { indirect, .. }
                | ParameterExpr::UppercasePattern { indirect, .. }
                | ParameterExpr::LowercaseFirstChar { indirect, .. }
                | ParameterExpr::LowercasePattern { indirect, .. }
                | ParameterExpr::ReplaceSubstring { indirect, .. } => *indirect,
                _ => false,
            };

            if indirect {
                result.has_indirection = true;
            }

            // Check if this is a pattern operation on an array parameter
            let param = match other {
                ParameterExpr::RemoveSmallestSuffixPattern { parameter, .. }
                | ParameterExpr::RemoveLargestSuffixPattern { parameter, .. }
                | ParameterExpr::RemoveSmallestPrefixPattern { parameter, .. }
                | ParameterExpr::RemoveLargestPrefixPattern { parameter, .. }
                | ParameterExpr::ReplaceSubstring { parameter, .. } => Some(parameter),
                _ => None,
            };

            if let Some(p) = param {
                if is_array_all_indices_param(p) {
                    result.has_array_at_expansion = true;
                }
            }
        }
    }

    // Check for quoted default values
    if has_quoted_default_value(expr) {
        result.has_quoted = true;
    }
}

/// Analyze word pieces for expansion behavior
pub fn analyze_word_pieces(pieces: &[WordPieceWithSource]) -> WordPartsAnalysis {
    let mut result = WordPartsAnalysis::default();

    for piece_with_source in pieces {
        analyze_word_piece(&piece_with_source.piece, &mut result);
    }

    result
}

/// Analyze a single word piece
fn analyze_word_piece(piece: &WordPiece, result: &mut WordPartsAnalysis) {
    match piece {
        WordPiece::SingleQuotedText(_) | WordPiece::AnsiCQuotedText(_) => {
            result.has_quoted = true;
        }
        WordPiece::DoubleQuotedSequence(inner_pieces) => {
            result.has_quoted = true;
            for inner in inner_pieces {
                match &inner.piece {
                    WordPiece::ParameterExpansion(expr) => {
                        analyze_parameter_expr(expr, result);
                    }
                    _ => {}
                }
            }
        }
        WordPiece::CommandSubstitution(_) | WordPiece::BackquotedCommandSubstitution(_) => {
            result.has_command_sub = true;
        }
        WordPiece::ParameterExpansion(expr) => {
            result.has_param_expansion = true;
            // Check if the parameter is $@ or $*
            match expr {
                ParameterExpr::Parameter { parameter, .. } if is_positional_all_param(parameter) => {
                    result.has_array_var = true;
                }
                _ => {}
            }
            analyze_parameter_expr(expr, result);
        }
        WordPiece::Text(t) => {
            if glob_pattern_has_var_ref(t) {
                result.has_param_expansion = true;
            }
        }
        _ => {}
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_pattern_has_var_ref() {
        assert!(glob_pattern_has_var_ref("$var"));
        assert!(glob_pattern_has_var_ref("${var}"));
        assert!(glob_pattern_has_var_ref("prefix$var"));
        assert!(glob_pattern_has_var_ref("+($ABC)"));
        assert!(!glob_pattern_has_var_ref("plain"));
        assert!(!glob_pattern_has_var_ref(r"\$var")); // escaped
        assert!(!glob_pattern_has_var_ref("$1")); // not a var name
    }
}
