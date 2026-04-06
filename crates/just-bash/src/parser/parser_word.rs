//! Word parsing logic for the Parser.
//!
//! Contains word token recognition, word parsing from tokens and strings,
//! expansion context creation, and command/backtick/arithmetic substitution helpers.

use crate::ast::types::{ArithmeticExpressionNode, WordNode, WordPart, AST};
use crate::parser::expansion_parser::{parse_word_parts, ExpansionContext};
use crate::parser::parser_substitution::{
    parse_backtick_substitution_from_string, parse_command_substitution_from_string, ErrorFn,
    ParserFactory,
};
use crate::parser::word_parser::parse_arith_expr_from_string;

use super::parser::Parser;
use crate::parser::lexer::TokenType;
use crate::parser::types::ParseException;

// ---------------------------------------------------------------------------
// Standalone helper functions (don't require Parser self)
// ---------------------------------------------------------------------------

/// Parse command substitution from a string - standalone version for ExpansionContext
pub(super) fn standalone_parse_command_substitution(
    value: &str,
    start: usize,
) -> (Option<WordPart>, usize) {
    let create_parser: ParserFactory = |input| {
        let mut p = Parser::new();
        p.parse(input).unwrap_or_else(|_| AST::script(vec![]))
    };
    let error: ErrorFn = |msg| {
        eprintln!("Parse error: {}", msg);
    };

    let (part, end_idx) =
        parse_command_substitution_from_string(value, start, create_parser, error);
    (Some(WordPart::CommandSubstitution(part)), end_idx)
}

/// Parse backtick substitution from a string - standalone version for ExpansionContext
pub(super) fn standalone_parse_backtick_substitution(
    value: &str,
    start: usize,
    in_double_quotes: bool,
) -> (WordPart, usize) {
    let create_parser: ParserFactory = |input| {
        let mut p = Parser::new();
        p.parse(input).unwrap_or_else(|_| AST::script(vec![]))
    };
    let error: ErrorFn = |msg| {
        eprintln!("Parse error: {}", msg);
    };

    let (part, end_idx) = parse_backtick_substitution_from_string(
        value,
        start,
        in_double_quotes,
        create_parser,
        error,
    );
    (WordPart::CommandSubstitution(part), end_idx)
}

/// Parse arithmetic expansion from a string - standalone version for ExpansionContext
pub(super) fn standalone_parse_arithmetic_expansion(
    value: &str,
    start: usize,
) -> (Option<WordPart>, usize) {
    let chars: Vec<char> = value.chars().collect();
    let expr_start = start + 3;
    let mut arith_depth = 1;
    let mut paren_depth = 0;
    let mut i = expr_start;

    while i < value.len().saturating_sub(1) && arith_depth > 0 {
        if i + 3 <= value.len() && &value[i..i + 3] == "$((" {
            arith_depth += 1;
            i += 3;
        } else if i + 2 <= value.len() && &value[i..i + 2] == "$(" {
            paren_depth += 1;
            i += 2;
        } else if i + 2 <= value.len() && &value[i..i + 2] == "))" {
            if paren_depth > 0 {
                paren_depth -= 1;
                i += 1;
            } else {
                arith_depth -= 1;
                if arith_depth > 0 {
                    i += 2;
                }
            }
        } else if chars.get(i) == Some(&'(') {
            paren_depth += 1;
            i += 1;
        } else if chars.get(i) == Some(&')') {
            if paren_depth > 0 {
                paren_depth -= 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }

    let expr_str: String = value
        .chars()
        .skip(expr_start)
        .take(i - expr_start)
        .collect();
    let expression = parse_arith_expr_from_string(&expr_str);

    (
        Some(WordPart::ArithmeticExpansion(
            crate::ast::types::ArithmeticExpansionPart { expression },
        )),
        i + 2,
    )
}

/// Check if $(( is a subshell - standalone version for ExpansionContext
pub(super) fn standalone_is_dollar_dparen_subshell(value: &str, start: usize) -> bool {
    crate::parser::parser_substitution::is_dollar_dparen_subshell(value, start)
}

/// Create an ExpansionContext with the standalone parsing functions
pub(super) fn create_expansion_context<'a>() -> ExpansionContext<'a> {
    ExpansionContext {
        parse_command_substitution: &standalone_parse_command_substitution,
        parse_backtick_substitution: &standalone_parse_backtick_substitution,
        parse_arithmetic_expansion: &standalone_parse_arithmetic_expansion,
        is_dollar_dparen_subshell: &standalone_is_dollar_dparen_subshell,
        report_error: &|msg| eprintln!("Parse error: {}", msg),
    }
}

// ---------------------------------------------------------------------------
// Parser impl: word parsing methods
// ---------------------------------------------------------------------------

impl Parser {
    pub fn is_word(&self) -> bool {
        let t = self.current().token_type;
        matches!(
            t,
            TokenType::Word
                | TokenType::Name
                | TokenType::Number
                | TokenType::If
                | TokenType::For
                | TokenType::While
                | TokenType::Until
                | TokenType::Case
                | TokenType::Function
                | TokenType::Else
                | TokenType::Elif
                | TokenType::Fi
                | TokenType::Then
                | TokenType::Do
                | TokenType::Done
                | TokenType::Esac
                | TokenType::In
                | TokenType::Select
                | TokenType::Time
                | TokenType::Coproc
                | TokenType::Bang
        )
    }

    pub fn parse_word(&mut self) -> Result<WordNode, ParseException> {
        let token = self.advance();
        Ok(self.parse_word_from_string(
            &token.value,
            token.quoted,
            token.single_quoted,
            false,
            false,
        ))
    }

    pub fn parse_word_no_brace_expansion(&mut self) -> Result<WordNode, ParseException> {
        let token = self.advance();
        Ok(self.parse_word_from_string(
            &token.value,
            token.quoted,
            token.single_quoted,
            false,
            false,
        ))
    }

    pub fn parse_word_for_regex(&mut self) -> Result<WordNode, ParseException> {
        let token = self.advance();
        Ok(self.parse_word_from_string(
            &token.value,
            token.quoted,
            token.single_quoted,
            false,
            false,
        ))
    }

    pub fn parse_word_from_string(
        &mut self,
        value: &str,
        quoted: bool,
        single_quoted: bool,
        is_assignment: bool,
        here_doc: bool,
    ) -> WordNode {
        self.parse_word_from_string_full(
            value,
            quoted,
            single_quoted,
            is_assignment,
            here_doc,
            false,
            false,
        )
    }

    /// Parse a word from string with all options
    pub fn parse_word_from_string_full(
        &mut self,
        value: &str,
        quoted: bool,
        single_quoted: bool,
        is_assignment: bool,
        here_doc: bool,
        no_brace_expansion: bool,
        regex_pattern: bool,
    ) -> WordNode {
        let ctx = create_expansion_context();
        let parts = parse_word_parts(
            &ctx,
            value,
            quoted,
            single_quoted,
            is_assignment,
            here_doc,
            false, // single_quotes_are_literal
            no_brace_expansion,
            regex_pattern,
            false, // in_parameter_expansion
        );
        AST::word(parts)
    }

    pub(super) fn do_parse_command_substitution(
        &mut self,
        value: &str,
        start: usize,
    ) -> (Option<crate::ast::types::WordPart>, usize) {
        standalone_parse_command_substitution(value, start)
    }

    pub(super) fn do_parse_backtick_substitution(
        &mut self,
        value: &str,
        start: usize,
        in_double_quotes: bool,
    ) -> (crate::ast::types::WordPart, usize) {
        standalone_parse_backtick_substitution(value, start, in_double_quotes)
    }

    pub(super) fn do_parse_arithmetic_expansion(
        &mut self,
        value: &str,
        start: usize,
    ) -> (Option<crate::ast::types::WordPart>, usize) {
        standalone_parse_arithmetic_expansion(value, start)
    }

    pub fn parse_command_substitution(
        &mut self,
        value: &str,
        start: usize,
    ) -> Result<(crate::ast::types::CommandSubstitutionPart, usize), ParseException> {
        let create_parser: ParserFactory = |input| {
            let mut p = Parser::new();
            p.parse(input).unwrap_or_else(|_| AST::script(vec![]))
        };
        let error: ErrorFn = |msg| {
            panic!("Parse error: {}", msg);
        };
        Ok(parse_command_substitution_from_string(
            value,
            start,
            create_parser,
            error,
        ))
    }

    pub fn parse_backtick_substitution(
        &mut self,
        value: &str,
        start: usize,
        in_double_quotes: bool,
    ) -> Result<(crate::ast::types::CommandSubstitutionPart, usize), ParseException> {
        let create_parser: ParserFactory = |input| {
            let mut p = Parser::new();
            p.parse(input).unwrap_or_else(|_| AST::script(vec![]))
        };
        let error: ErrorFn = |msg| {
            panic!("Parse error: {}", msg);
        };
        Ok(parse_backtick_substitution_from_string(
            value,
            start,
            in_double_quotes,
            create_parser,
            error,
        ))
    }

    pub fn is_dollar_dparen_subshell(&self, value: &str, start: usize) -> bool {
        crate::parser::parser_substitution::is_dollar_dparen_subshell(value, start)
    }

    pub fn parse_arithmetic_expansion(
        &mut self,
        value: &str,
        start: usize,
    ) -> Result<(crate::ast::types::ArithmeticExpansionPart, usize), ParseException> {
        let chars: Vec<char> = value.chars().collect();
        let expr_start = start + 3;
        let mut arith_depth = 1;
        let mut paren_depth = 0;
        let mut i = expr_start;

        while i < value.len().saturating_sub(1) && arith_depth > 0 {
            if value[i..].starts_with("$((") {
                arith_depth += 1;
                i += 3;
            } else if value[i..].starts_with("$((") {
                paren_depth += 1;
                i += 2;
            } else if value[i..].starts_with("))") {
                if paren_depth > 0 {
                    paren_depth -= 1;
                    i += 1;
                } else {
                    arith_depth -= 1;
                    if arith_depth > 0 {
                        i += 2;
                    }
                }
            } else if value.chars().nth(i) == Some('(') {
                paren_depth += 1;
                i += 1;
            } else if value.chars().nth(i) == Some(')') {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                i += 1;
            } else {
                i += 1;
            }
        }

        let expr_str: String = value
            .chars()
            .skip(expr_start)
            .take(i - expr_start)
            .collect();
        let expression = parse_arith_expr_from_string(&expr_str);

        Ok((
            crate::ast::types::ArithmeticExpansionPart { expression },
            i + 2,
        ))
    }

    pub fn parse_arithmetic_expression(&mut self, input: &str) -> ArithmeticExpressionNode {
        parse_arith_expr_from_string(input)
    }
}
