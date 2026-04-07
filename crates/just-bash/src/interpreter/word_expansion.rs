//! Word Expansion
//!
//! Main entry point for shell word expansion.
//!
//! Handles shell word expansion including:
//! - Variable expansion ($VAR, ${VAR})
//! - Command substitution $(...)
//! - Arithmetic expansion $((...)
//! - Tilde expansion (~)
//! - Brace expansion {a,b,c}
//! - Glob expansion (*, ?, [...])
//!
//! This module provides the high-level expansion functions.
//! The actual expansion logic is implemented in the expansion/ submodules.
//! Command substitution requires runtime dependencies (script execution).

use brush_parser::ast as bast;
use brush_parser::word::{WordPiece, WordPieceWithSource, ParameterExpr, Parameter, SpecialParameter};
use brush_parser::ParserOptions;

use crate::interpreter::interpreter::FileSystem as SyncInterpreterFs;
use crate::interpreter::types::{ExecResult, InterpreterState};

// Re-export commonly used expansion functions
pub use crate::interpreter::expansion::analysis::*;
pub use crate::interpreter::expansion::brace_range::*;
pub use crate::interpreter::expansion::glob_escape::*;
pub use crate::interpreter::expansion::pattern::*;
pub use crate::interpreter::expansion::pattern_removal::*;
pub use crate::interpreter::expansion::quoting::*;
pub use crate::interpreter::expansion::tilde::*;
pub use crate::interpreter::expansion::variable::*;
pub use crate::interpreter::expansion::word_split::*;

/// Result of word expansion.
#[derive(Debug, Clone)]
pub struct WordExpansionResult {
    /// The expanded string value
    pub value: String,
    /// Whether the expansion produced multiple words (from word splitting)
    pub split_words: Option<Vec<String>>,
    /// Any stderr output from command substitutions
    pub stderr: String,
    /// Exit code from command substitutions (if any)
    pub exit_code: Option<i32>,
}

impl WordExpansionResult {
    /// Create a simple result with just a value.
    pub fn simple(value: String) -> Self {
        Self {
            value,
            split_words: None,
            stderr: String::new(),
            exit_code: None,
        }
    }

    /// Create a result with split words.
    pub fn with_split(value: String, words: Vec<String>) -> Self {
        Self {
            value,
            split_words: Some(words),
            stderr: String::new(),
            exit_code: None,
        }
    }
}

/// Options for word expansion.
#[derive(Debug, Clone, Default)]
pub struct WordExpansionOptions {
    /// Whether we're inside double quotes
    pub in_double_quotes: bool,
    /// Whether to perform word splitting
    pub do_word_split: bool,
    /// Whether to perform glob expansion
    pub do_glob: bool,
    /// Whether to preserve empty fields
    pub preserve_empty: bool,
    /// Whether extglob is enabled
    pub extglob: bool,
}

/// Callback type for command substitution execution.
///
/// The runtime must provide this callback to execute command substitutions.
/// It takes the command string and returns the execution result.
pub type CommandSubstitutionFn =
    Box<dyn Fn(&str, &mut InterpreterState) -> ExecResult + Send + Sync>;

/// Callback type for command substitution (reference version).
///
/// This is the signature used by the public API functions.
/// The callback receives the command body and mutable state, returns (output, exit_code).
pub type CommandSubstFn<'a> = &'a dyn Fn(&str, &mut InterpreterState) -> (String, i32);

/// Parse a bast::Word into WordPiece list on demand.
fn parse_word(word: &bast::Word) -> Vec<WordPieceWithSource> {
    let options = ParserOptions::default();
    brush_parser::word::parse(&word.value, &options).unwrap_or_default()
}

/// Extract a parameter name from a brush_parser Parameter for simple variable lookup.
fn parameter_name(param: &Parameter) -> String {
    match param {
        Parameter::Named(name) => name.clone(),
        Parameter::Positional(n) => n.to_string(),
        Parameter::Special(sp) => match sp {
            SpecialParameter::AllPositionalParameters { concatenate } => {
                if *concatenate { "*".to_string() } else { "@".to_string() }
            }
            SpecialParameter::PositionalParameterCount => "#".to_string(),
            SpecialParameter::LastExitStatus => "?".to_string(),
            SpecialParameter::CurrentOptionFlags => "-".to_string(),
            SpecialParameter::ProcessId => "$".to_string(),
            SpecialParameter::LastBackgroundProcessId => "!".to_string(),
            SpecialParameter::ShellName => "0".to_string(),
        },
        Parameter::NamedWithIndex { name, index } => format!("{}[{}]", name, index),
        Parameter::NamedWithAllIndices { name, concatenate } => {
            if *concatenate {
                format!("{}[*]", name)
            } else {
                format!("{}[@]", name)
            }
        }
    }
}

/// Expand a word without glob expansion.
///
/// This performs all expansions except glob expansion:
/// - Tilde expansion
/// - Parameter expansion
/// - Command substitution (requires callback)
/// - Arithmetic expansion
/// - Brace expansion
/// - Quote removal
///
/// For command substitution, if no callback is provided, $(...) and `...`
/// are left unexpanded.
pub fn expand_word_no_glob(
    state: &InterpreterState,
    word: &bast::Word,
    options: &WordExpansionOptions,
) -> WordExpansionResult {
    let pieces = parse_word(word);
    let mut result = String::new();

    for pws in &pieces {
        result.push_str(&expand_piece_no_glob(state, &pws.piece, options));
    }

    WordExpansionResult::simple(result)
}

/// Expand a single word piece without glob expansion.
fn expand_piece_no_glob(
    state: &InterpreterState,
    piece: &WordPiece,
    options: &WordExpansionOptions,
) -> String {
    use crate::interpreter::expansion::tilde::apply_tilde_expansion;
    use crate::interpreter::expansion::variable::get_variable;

    match piece {
        WordPiece::Text(text) => text.clone(),
        WordPiece::SingleQuotedText(text) => text.clone(),
        WordPiece::AnsiCQuotedText(text) => text.clone(),
        WordPiece::EscapeSequence(text) => text.clone(),
        WordPiece::TildePrefix(prefix) => {
            // Tilde expansion doesn't happen inside double quotes
            if options.in_double_quotes {
                return prefix.clone();
            }
            apply_tilde_expansion(state, prefix)
        }
        WordPiece::ParameterExpansion(expr) => {
            // Simple variable expansion via parameter name
            let name = get_parameter_name_from_expr(expr);
            get_variable(state, &name)
        }
        WordPiece::DoubleQuotedSequence(inner_pieces) => {
            let inner_options = WordExpansionOptions {
                in_double_quotes: true,
                ..options.clone()
            };
            let mut result = String::new();
            for inner_pws in inner_pieces {
                result.push_str(&expand_piece_no_glob(state, &inner_pws.piece, &inner_options));
            }
            result
        }
        WordPiece::GettextDoubleQuotedSequence(inner_pieces) => {
            // Treat same as double-quoted
            let inner_options = WordExpansionOptions {
                in_double_quotes: true,
                ..options.clone()
            };
            let mut result = String::new();
            for inner_pws in inner_pieces {
                result.push_str(&expand_piece_no_glob(state, &inner_pws.piece, &inner_options));
            }
            result
        }
        WordPiece::CommandSubstitution(_) | WordPiece::BackquotedCommandSubstitution(_) => {
            // Command substitution requires runtime callback
            // Return empty string if no callback provided
            String::new()
        }
        WordPiece::ArithmeticExpression(arith) => {
            use crate::interpreter::arithmetic::evaluate_arithmetic;
            use crate::interpreter::types::{ExecutionLimits, InterpreterContext};

            let limits = ExecutionLimits::default();
            let mut state_clone = state.clone();
            let mut ctx = InterpreterContext::new(&mut state_clone, &limits);
            match evaluate_arithmetic(&mut ctx, &arith.value, false, None) {
                Ok(value) => value.to_string(),
                Err(_) => "0".to_string(),
            }
        }
    }
}

/// Extract the parameter name from a ParameterExpr for simple variable lookup.
fn get_parameter_name_from_expr(expr: &ParameterExpr) -> String {
    match expr {
        ParameterExpr::Parameter { parameter, .. } => parameter_name(parameter),
        ParameterExpr::UseDefaultValues { parameter, .. } => parameter_name(parameter),
        ParameterExpr::AssignDefaultValues { parameter, .. } => parameter_name(parameter),
        ParameterExpr::IndicateErrorIfNullOrUnset { parameter, .. } => parameter_name(parameter),
        ParameterExpr::UseAlternativeValue { parameter, .. } => parameter_name(parameter),
        ParameterExpr::ParameterLength { parameter, .. } => parameter_name(parameter),
        ParameterExpr::RemoveSmallestSuffixPattern { parameter, .. } => parameter_name(parameter),
        ParameterExpr::RemoveLargestSuffixPattern { parameter, .. } => parameter_name(parameter),
        ParameterExpr::RemoveSmallestPrefixPattern { parameter, .. } => parameter_name(parameter),
        ParameterExpr::RemoveLargestPrefixPattern { parameter, .. } => parameter_name(parameter),
        ParameterExpr::Substring { parameter, .. } => parameter_name(parameter),
        ParameterExpr::Transform { parameter, .. } => parameter_name(parameter),
        ParameterExpr::UppercaseFirstChar { parameter, .. } => parameter_name(parameter),
        ParameterExpr::UppercasePattern { parameter, .. } => parameter_name(parameter),
        ParameterExpr::LowercaseFirstChar { parameter, .. } => parameter_name(parameter),
        ParameterExpr::LowercasePattern { parameter, .. } => parameter_name(parameter),
        ParameterExpr::ReplaceSubstring { parameter, .. } => parameter_name(parameter),
        ParameterExpr::VariableNames { prefix, .. } => prefix.clone(),
        ParameterExpr::MemberKeys { variable_name, .. } => variable_name.clone(),
    }
}

// ============================================================================
// Core Word Expansion Functions with Command Substitution Support
// ============================================================================

/// Expand a word to a single string.
///
/// This is the main entry point for word expansion. It performs all expansions:
/// - Tilde expansion
/// - Parameter expansion
/// - Command substitution (via callback)
/// - Arithmetic expansion
/// - Quote removal
///
/// Note: This does NOT perform word splitting or glob expansion.
/// Use `expand_word_with_glob` for full expansion including glob.
pub fn expand_word(
    state: &mut InterpreterState,
    word: &bast::Word,
    cmd_subst: Option<CommandSubstFn>,
) -> WordExpansionResult {
    let options = WordExpansionOptions::default();
    expand_word_with_options(state, word, &options, cmd_subst)
}

/// Expand a word with specific options.
pub fn expand_word_with_options(
    state: &mut InterpreterState,
    word: &bast::Word,
    options: &WordExpansionOptions,
    cmd_subst: Option<CommandSubstFn>,
) -> WordExpansionResult {
    let pieces = parse_word(word);
    let mut result = String::new();
    let mut stderr = String::new();
    let mut last_exit_code = None;

    for pws in &pieces {
        let (expanded, part_stderr, exit_code) =
            expand_piece_with_cmd_subst(state, &pws.piece, options, cmd_subst);
        result.push_str(&expanded);
        if !part_stderr.is_empty() {
            if !stderr.is_empty() {
                stderr.push('\n');
            }
            stderr.push_str(&part_stderr);
        }
        if exit_code.is_some() {
            last_exit_code = exit_code;
        }
    }

    WordExpansionResult {
        value: result,
        split_words: None,
        stderr,
        exit_code: last_exit_code,
    }
}

/// Expand a word for use as a regex pattern (in [[ =~ ]]).
///
/// Preserves backslash escapes so they're passed to the regex engine.
/// For example, \[\] becomes \[\] in the regex (matching literal [ and ]).
pub fn expand_word_for_regex(
    state: &mut InterpreterState,
    word: &bast::Word,
    cmd_subst: Option<CommandSubstFn>,
) -> WordExpansionResult {
    let pieces = parse_word(word);
    let mut result = String::new();
    let mut stderr = String::new();
    let mut last_exit_code = None;
    let options = WordExpansionOptions::default();

    for pws in &pieces {
        match &pws.piece {
            WordPiece::EscapeSequence(esc) => {
                // For regex patterns, preserve ALL backslash escapes
                // This allows \[ \] \. \* etc. to work as regex escapes
                result.push('\\');
                result.push_str(esc);
            }
            WordPiece::SingleQuotedText(text) => {
                // Single-quoted content is literal in regex
                result.push_str(text);
            }
            WordPiece::DoubleQuotedSequence(inner_pieces) => {
                // Double-quoted: expand contents
                let inner_options = WordExpansionOptions {
                    in_double_quotes: true,
                    ..options.clone()
                };
                for inner_pws in inner_pieces {
                    let (expanded, part_stderr, exit_code) =
                        expand_piece_with_cmd_subst(state, &inner_pws.piece, &inner_options, cmd_subst);
                    result.push_str(&expanded);
                    if !part_stderr.is_empty() {
                        stderr.push_str(&part_stderr);
                    }
                    if exit_code.is_some() {
                        last_exit_code = exit_code;
                    }
                }
            }
            WordPiece::TildePrefix(_) => {
                // Tilde expansion on RHS of =~ is treated as literal (regex chars escaped)
                let (expanded, part_stderr, exit_code) =
                    expand_piece_with_cmd_subst(state, &pws.piece, &options, cmd_subst);
                result.push_str(&escape_regex_chars(&expanded));
                if !part_stderr.is_empty() {
                    stderr.push_str(&part_stderr);
                }
                if exit_code.is_some() {
                    last_exit_code = exit_code;
                }
            }
            _ => {
                // Other pieces: expand normally
                let (expanded, part_stderr, exit_code) =
                    expand_piece_with_cmd_subst(state, &pws.piece, &options, cmd_subst);
                result.push_str(&expanded);
                if !part_stderr.is_empty() {
                    stderr.push_str(&part_stderr);
                }
                if exit_code.is_some() {
                    last_exit_code = exit_code;
                }
            }
        }
    }

    WordExpansionResult {
        value: result,
        split_words: None,
        stderr,
        exit_code: last_exit_code,
    }
}

/// Expand a word for use as a pattern (e.g., in [[ == ]] or case).
///
/// Preserves backslash escapes for pattern metacharacters so they're treated literally.
/// This prevents `*\(\)` from being interpreted as an extglob pattern.
pub fn expand_word_for_pattern(
    state: &mut InterpreterState,
    word: &bast::Word,
    cmd_subst: Option<CommandSubstFn>,
) -> WordExpansionResult {
    let pieces = parse_word(word);
    let mut result = String::new();
    let mut stderr = String::new();
    let mut last_exit_code = None;
    let options = WordExpansionOptions::default();

    for pws in &pieces {
        match &pws.piece {
            WordPiece::EscapeSequence(esc) => {
                // For escaped characters that are pattern metacharacters, preserve the backslash
                // This includes: ( ) | * ? [ ] for glob/extglob patterns
                if "()|*?[]".contains(esc.as_str()) {
                    result.push('\\');
                    result.push_str(esc);
                } else {
                    result.push_str(esc);
                }
            }
            WordPiece::SingleQuotedText(text) => {
                // Single-quoted content should be escaped for literal matching
                result.push_str(&escape_glob_chars(text));
            }
            WordPiece::DoubleQuotedSequence(inner_pieces) => {
                // Double-quoted: expand contents and escape for literal matching
                let inner_options = WordExpansionOptions {
                    in_double_quotes: true,
                    ..options.clone()
                };
                let mut inner_result = String::new();
                for inner_pws in inner_pieces {
                    let (expanded, part_stderr, exit_code) =
                        expand_piece_with_cmd_subst(state, &inner_pws.piece, &inner_options, cmd_subst);
                    inner_result.push_str(&expanded);
                    if !part_stderr.is_empty() {
                        stderr.push_str(&part_stderr);
                    }
                    if exit_code.is_some() {
                        last_exit_code = exit_code;
                    }
                }
                result.push_str(&escape_glob_chars(&inner_result));
            }
            _ => {
                // Other pieces: expand normally
                let (expanded, part_stderr, exit_code) =
                    expand_piece_with_cmd_subst(state, &pws.piece, &options, cmd_subst);
                result.push_str(&expanded);
                if !part_stderr.is_empty() {
                    stderr.push_str(&part_stderr);
                }
                if exit_code.is_some() {
                    last_exit_code = exit_code;
                }
            }
        }
    }

    WordExpansionResult {
        value: result,
        split_words: None,
        stderr,
        exit_code: last_exit_code,
    }
}

/// Expand a word and perform glob expansion.
///
/// This performs full word expansion including glob/pathname expansion.
/// Returns multiple values if glob expansion produces matches.
///
/// When `fs` is [`Some`], pathname expansion uses the virtual [`SyncInterpreterFs`]
/// (via [`SyncInterpreterFs::glob`]) so globs never touch the host OS. When `fs` is
/// [`None`], expansion falls back to the real filesystem (for isolated unit tests).
pub fn expand_word_with_glob(
    state: &mut InterpreterState,
    word: &bast::Word,
    cmd_subst: Option<CommandSubstFn>,
    fs: Option<&dyn SyncInterpreterFs>,
) -> WordExpansionResult {
    use crate::interpreter::expansion::glob_escape::unescape_glob_pattern;
    use crate::interpreter::expansion::word_glob_expansion::expand_glob_pattern;
    use std::path::Path;

    // First, expand the word for glob matching
    let pattern = expand_word_for_globbing(state, word, cmd_subst);

    // Check if we should do glob expansion
    let noglob = state.options.noglob;
    let extglob = state.shopt_options.extglob;

    if noglob || !has_glob_pattern(&pattern.value, extglob) {
        // No glob expansion needed - return the expanded value
        return pattern;
    }

    let failglob = state.shopt_options.failglob;
    let nullglob = state.shopt_options.nullglob;
    let cwd = state.cwd.clone();

    if let Some(fs) = fs {
        match fs.glob(&pattern.value, &cwd) {
            Ok(mut values) => {
                values.sort();
                if values.is_empty() {
                    if failglob {
                        let stderr = if pattern.stderr.is_empty() {
                            format!("no match: {}", &pattern.value)
                        } else {
                            format!("{}\nno match: {}", pattern.stderr, &pattern.value)
                        };
                        return WordExpansionResult {
                            value: pattern.value,
                            split_words: None,
                            stderr,
                            exit_code: Some(1),
                        };
                    }
                    if nullglob {
                        return WordExpansionResult {
                            value: String::new(),
                            split_words: Some(vec![]),
                            stderr: pattern.stderr,
                            exit_code: pattern.exit_code,
                        };
                    }
                    return WordExpansionResult {
                        value: unescape_glob_pattern(&pattern.value),
                        split_words: None,
                        stderr: pattern.stderr,
                        exit_code: pattern.exit_code,
                    };
                }
                if values.len() == 1 {
                    WordExpansionResult {
                        value: values.into_iter().next().unwrap_or_default(),
                        split_words: None,
                        stderr: pattern.stderr,
                        exit_code: pattern.exit_code,
                    }
                } else {
                    let first = values.first().cloned().unwrap_or_default();
                    WordExpansionResult {
                        value: first,
                        split_words: Some(values),
                        stderr: pattern.stderr,
                        exit_code: pattern.exit_code,
                    }
                }
            }
            Err(e) => {
                if failglob {
                    return WordExpansionResult {
                        value: pattern.value,
                        split_words: None,
                        stderr: if pattern.stderr.is_empty() {
                            e.to_string()
                        } else {
                            format!("{}\n{}", pattern.stderr, e)
                        },
                        exit_code: Some(1),
                    };
                }
                WordExpansionResult {
                    value: unescape_glob_pattern(&pattern.value),
                    split_words: None,
                    stderr: pattern.stderr,
                    exit_code: pattern.exit_code,
                }
            }
        }
    } else {
        let cwd_path = Path::new(&cwd);
        match expand_glob_pattern(&pattern.value, cwd_path, failglob, nullglob, extglob) {
            Ok(glob_result) => {
                if glob_result.values.len() == 1 {
                    WordExpansionResult {
                        value: glob_result.values.into_iter().next().unwrap_or_default(),
                        split_words: None,
                        stderr: pattern.stderr,
                        exit_code: pattern.exit_code,
                    }
                } else {
                    let first = glob_result.values.first().cloned().unwrap_or_default();
                    WordExpansionResult {
                        value: first,
                        split_words: Some(glob_result.values),
                        stderr: pattern.stderr,
                        exit_code: pattern.exit_code,
                    }
                }
            }
            Err(e) => WordExpansionResult {
                value: pattern.value,
                split_words: None,
                stderr: if pattern.stderr.is_empty() {
                    e
                } else {
                    format!("{}\n{}", pattern.stderr, e)
                },
                exit_code: Some(1),
            },
        }
    }
}

/// Expand a word for glob matching.
///
/// Unlike regular expansion, this escapes glob metacharacters in quoted parts
/// so they are treated as literals, while preserving glob patterns from unquoted text.
fn expand_word_for_globbing(
    state: &mut InterpreterState,
    word: &bast::Word,
    cmd_subst: Option<CommandSubstFn>,
) -> WordExpansionResult {
    use crate::interpreter::expansion::pattern_expansion::expand_variables_in_pattern;

    let pieces = parse_word(word);
    let mut result = String::new();
    let mut stderr = String::new();
    let mut last_exit_code = None;
    let options = WordExpansionOptions::default();

    for pws in &pieces {
        match &pws.piece {
            WordPiece::SingleQuotedText(text) => {
                // Single-quoted content: escape glob metacharacters for literal matching
                result.push_str(&escape_glob_chars(text));
            }
            WordPiece::EscapeSequence(esc) => {
                // Escaped character: escape if it's a glob metacharacter
                if "*?[]\\()|".contains(esc.as_str()) {
                    result.push('\\');
                    result.push_str(esc);
                } else {
                    result.push_str(esc);
                }
            }
            WordPiece::DoubleQuotedSequence(inner_pieces) => {
                // Double-quoted: expand contents and escape glob metacharacters
                let inner_options = WordExpansionOptions {
                    in_double_quotes: true,
                    ..options.clone()
                };
                let mut inner_result = String::new();
                for inner_pws in inner_pieces {
                    let (expanded, part_stderr, exit_code) =
                        expand_piece_with_cmd_subst(state, &inner_pws.piece, &inner_options, cmd_subst);
                    inner_result.push_str(&expanded);
                    if !part_stderr.is_empty() {
                        stderr.push_str(&part_stderr);
                    }
                    if exit_code.is_some() {
                        last_exit_code = exit_code;
                    }
                }
                result.push_str(&escape_glob_chars(&inner_result));
            }
            WordPiece::Text(text) => {
                // Unquoted text: may contain glob characters that should glob,
                // and may contain variables in extglob patterns
                result.push_str(&expand_variables_in_pattern(state, text));
            }
            _ => {
                // Other pieces (ParameterExpansion, etc.): expand normally
                let (expanded, part_stderr, exit_code) =
                    expand_piece_with_cmd_subst(state, &pws.piece, &options, cmd_subst);
                result.push_str(&expanded);
                if !part_stderr.is_empty() {
                    stderr.push_str(&part_stderr);
                }
                if exit_code.is_some() {
                    last_exit_code = exit_code;
                }
            }
        }
    }

    WordExpansionResult {
        value: result,
        split_words: None,
        stderr,
        exit_code: last_exit_code,
    }
}

/// Expand a single word piece with command substitution support.
///
/// Returns (expanded_value, stderr, exit_code).
fn expand_piece_with_cmd_subst(
    state: &mut InterpreterState,
    piece: &WordPiece,
    options: &WordExpansionOptions,
    cmd_subst: Option<CommandSubstFn>,
) -> (String, String, Option<i32>) {
    use crate::interpreter::expansion::tilde::apply_tilde_expansion;
    use crate::interpreter::expansion::variable::get_variable;

    match piece {
        WordPiece::Text(text) => (text.clone(), String::new(), None),
        WordPiece::SingleQuotedText(text) => (text.clone(), String::new(), None),
        WordPiece::AnsiCQuotedText(text) => (text.clone(), String::new(), None),
        WordPiece::EscapeSequence(text) => (text.clone(), String::new(), None),
        WordPiece::TildePrefix(prefix) => {
            // Tilde expansion doesn't happen inside double quotes
            if options.in_double_quotes {
                return (prefix.clone(), String::new(), None);
            }
            (
                apply_tilde_expansion(state, prefix),
                String::new(),
                None,
            )
        }
        WordPiece::ParameterExpansion(expr) => {
            // Variable expansion via parameter name
            let name = get_parameter_name_from_expr(expr);
            (get_variable(state, &name), String::new(), None)
        }
        WordPiece::DoubleQuotedSequence(inner_pieces) => {
            let inner_options = WordExpansionOptions {
                in_double_quotes: true,
                ..options.clone()
            };
            let mut result = String::new();
            let mut stderr = String::new();
            let mut last_exit_code = None;
            for inner_pws in inner_pieces {
                let (expanded, part_stderr, exit_code) =
                    expand_piece_with_cmd_subst(state, &inner_pws.piece, &inner_options, cmd_subst);
                result.push_str(&expanded);
                if !part_stderr.is_empty() {
                    stderr.push_str(&part_stderr);
                }
                if exit_code.is_some() {
                    last_exit_code = exit_code;
                }
            }
            (result, stderr, last_exit_code)
        }
        WordPiece::GettextDoubleQuotedSequence(inner_pieces) => {
            // Treat same as double-quoted
            let inner_options = WordExpansionOptions {
                in_double_quotes: true,
                ..options.clone()
            };
            let mut result = String::new();
            let mut stderr = String::new();
            let mut last_exit_code = None;
            for inner_pws in inner_pieces {
                let (expanded, part_stderr, exit_code) =
                    expand_piece_with_cmd_subst(state, &inner_pws.piece, &inner_options, cmd_subst);
                result.push_str(&expanded);
                if !part_stderr.is_empty() {
                    stderr.push_str(&part_stderr);
                }
                if exit_code.is_some() {
                    last_exit_code = exit_code;
                }
            }
            (result, stderr, last_exit_code)
        }
        WordPiece::CommandSubstitution(body) => {
            // Command substitution requires the callback
            if let Some(callback) = cmd_subst {
                let (output, exit_code) = callback(body, state);
                // Remove trailing newlines (bash behavior)
                let trimmed = output.trim_end_matches('\n').to_string();
                (trimmed, String::new(), Some(exit_code))
            } else {
                (String::new(), String::new(), None)
            }
        }
        WordPiece::BackquotedCommandSubstitution(body) => {
            // Backtick command substitution - same behavior as $()
            if let Some(callback) = cmd_subst {
                let (output, exit_code) = callback(body, state);
                let trimmed = output.trim_end_matches('\n').to_string();
                (trimmed, String::new(), Some(exit_code))
            } else {
                (String::new(), String::new(), None)
            }
        }
        WordPiece::ArithmeticExpression(arith) => {
            use crate::interpreter::arithmetic::evaluate_arithmetic;
            use crate::interpreter::types::{ExecutionLimits, InterpreterContext};

            let limits = ExecutionLimits::default();
            let mut ctx = InterpreterContext::new(state, &limits);
            match evaluate_arithmetic(&mut ctx, &arith.value, false, None) {
                Ok(value) => (value.to_string(), String::new(), None),
                Err(_) => ("0".to_string(), String::new(), None),
            }
        }
    }
}

// ============================================================================
// Word Analysis Functions
// ============================================================================

/// Check if a word is "fully quoted" - meaning glob characters should be treated literally.
///
/// A word is fully quoted if all its pieces are either:
/// - SingleQuotedText
/// - DoubleQuotedSequence (entirely quoted variable expansion like "$pat")
/// - EscapeSequence
/// - AnsiCQuotedText
pub fn is_word_fully_quoted(word: &bast::Word) -> bool {
    let pieces = parse_word(word);

    // Empty word is considered quoted (matches empty pattern literally)
    if pieces.is_empty() {
        return true;
    }

    for pws in &pieces {
        match &pws.piece {
            WordPiece::SingleQuotedText(_)
            | WordPiece::AnsiCQuotedText(_)
            | WordPiece::DoubleQuotedSequence(_)
            | WordPiece::GettextDoubleQuotedSequence(_)
            | WordPiece::EscapeSequence(_) => {}
            _ => return false,
        }
    }
    true
}

/// Check if a word contains any glob patterns.
pub fn word_has_glob_pattern(word: &bast::Word, extglob: bool) -> bool {
    use crate::interpreter::expansion::glob_escape::has_glob_pattern;

    let pieces = parse_word(word);

    for pws in &pieces {
        if let WordPiece::Text(text) = &pws.piece {
            if has_glob_pattern(text, extglob) {
                return true;
            }
        }
    }
    false
}

/// Check if a word contains command substitution.
pub fn word_has_command_substitution(word: &bast::Word) -> bool {
    let pieces = parse_word(word);

    for pws in &pieces {
        match &pws.piece {
            WordPiece::CommandSubstitution(_) | WordPiece::BackquotedCommandSubstitution(_) => {
                return true;
            }
            WordPiece::DoubleQuotedSequence(inner) => {
                for inner_pws in inner {
                    if matches!(
                        &inner_pws.piece,
                        WordPiece::CommandSubstitution(_) | WordPiece::BackquotedCommandSubstitution(_)
                    ) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_word(s: &str) -> bast::Word {
        bast::Word {
            value: s.to_string(),
            loc: None,
        }
    }

    #[test]
    fn test_expand_word_literal() {
        let state = InterpreterState::default();
        let word = make_word("hello");
        let options = WordExpansionOptions::default();
        let result = expand_word_no_glob(&state, &word, &options);
        assert_eq!(result.value, "hello");
    }

    #[test]
    fn test_expand_word_variable() {
        let mut state = InterpreterState::default();
        state.env.insert("FOO".to_string(), "bar".to_string());
        let word = make_word("$FOO");
        let options = WordExpansionOptions::default();
        let result = expand_word_no_glob(&state, &word, &options);
        assert_eq!(result.value, "bar");
    }

    #[test]
    fn test_expand_word_unset_variable() {
        let state = InterpreterState::default();
        let word = make_word("$UNSET");
        let options = WordExpansionOptions::default();
        let result = expand_word_no_glob(&state, &word, &options);
        assert_eq!(result.value, "");
    }

    #[test]
    fn test_is_word_fully_quoted_empty() {
        let word = make_word("");
        assert!(is_word_fully_quoted(&word));
    }

    #[test]
    fn test_is_word_fully_quoted_single_quoted() {
        let word = make_word("'hello'");
        assert!(is_word_fully_quoted(&word));
    }

    #[test]
    fn test_is_word_fully_quoted_literal() {
        let word = make_word("hello");
        assert!(!is_word_fully_quoted(&word));
    }

    #[test]
    fn test_word_has_glob_pattern() {
        let word = make_word("*.txt");
        assert!(word_has_glob_pattern(&word, false));

        let word = make_word("hello");
        assert!(!word_has_glob_pattern(&word, false));
    }

    #[test]
    fn test_word_has_command_substitution() {
        let word = make_word("$(echo hi)");
        assert!(word_has_command_substitution(&word));

        let word = make_word("hello");
        assert!(!word_has_command_substitution(&word));
    }

    // ============================================================================
    // Tests for Core Word Expansion Functions
    // ============================================================================

    #[test]
    fn test_expand_word_literal_with_cmd_subst() {
        let mut state = InterpreterState::default();
        let word = make_word("hello");
        let result = expand_word(&mut state, &word, None);
        assert_eq!(result.value, "hello");
    }

    #[test]
    fn test_expand_word_variable_with_cmd_subst() {
        let mut state = InterpreterState::default();
        state.env.insert("FOO".to_string(), "bar".to_string());
        let word = make_word("$FOO");
        let result = expand_word(&mut state, &word, None);
        assert_eq!(result.value, "bar");
    }

    #[test]
    fn test_expand_word_with_callback() {
        let mut state = InterpreterState::default();
        let word = make_word("$(echo hello)");

        // Callback that returns a fixed value
        let callback: CommandSubstFn =
            &|_cmd: &str, _state: &mut InterpreterState| ("hello from callback\n".to_string(), 0);

        let result = expand_word(&mut state, &word, Some(callback));
        assert_eq!(result.value, "hello from callback");
        assert_eq!(result.exit_code, Some(0));
    }

    #[test]
    fn test_expand_word_for_regex_preserves_escapes() {
        let mut state = InterpreterState::default();
        // Test that escaped chars are preserved with backslashes
        // brush-parser parses \[ as EscapeSequence("\\["), preserving the backslash
        let word = make_word("\\[");
        let result = expand_word_for_regex(&mut state, &word, None);
        assert_eq!(result.value, "\\\\[");
    }

    #[test]
    fn test_expand_word_for_regex_single_quoted() {
        let mut state = InterpreterState::default();
        let word = make_word("'[abc]'");
        let result = expand_word_for_regex(&mut state, &word, None);
        // Single-quoted content is literal
        assert_eq!(result.value, "[abc]");
    }

    #[test]
    fn test_expand_word_for_pattern_preserves_metachar_escapes() {
        let mut state = InterpreterState::default();
        // Pattern metacharacters should be preserved with backslash
        let word = make_word("\\*");
        let result = expand_word_for_pattern(&mut state, &word, None);
        assert_eq!(result.value, "\\*");
    }

    #[test]
    fn test_expand_word_for_pattern_escapes_single_quoted() {
        let mut state = InterpreterState::default();
        let word = make_word("'*?.txt'");
        let result = expand_word_for_pattern(&mut state, &word, None);
        // Glob chars should be escaped
        assert_eq!(result.value, "\\*\\?.txt");
    }

    #[test]
    fn test_expand_word_for_pattern_non_metachar_not_preserved() {
        let mut state = InterpreterState::default();
        // brush-parser parses \a as EscapeSequence("\\a"), preserving the backslash
        let word = make_word("\\a");
        let result = expand_word_for_pattern(&mut state, &word, None);
        assert_eq!(result.value, "\\a");
    }

    #[test]
    fn test_expand_word_with_glob_noglob() {
        let mut state = InterpreterState::default();
        state.options.noglob = true;

        let word = make_word("*.txt");
        let result = expand_word_with_glob(&mut state, &word, None, None);
        // With noglob, pattern should not be expanded
        assert_eq!(result.value, "*.txt");
    }

    #[test]
    fn test_expand_word_combined() {
        let mut state = InterpreterState::default();
        state.env.insert("NAME".to_string(), "world".to_string());

        // "hello $NAME"
        let word = make_word("hello $NAME");
        let result = expand_word(&mut state, &word, None);
        assert_eq!(result.value, "hello world");
    }
}
