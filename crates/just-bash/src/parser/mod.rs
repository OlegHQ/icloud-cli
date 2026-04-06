//! Parser module for bash scripts
//!
//! This module contains the lexer and parser for bash scripts.

pub mod arithmetic_parser;
pub mod arithmetic_primaries;
pub mod command_parser;
pub mod compound_parser;
pub mod conditional_parser;
pub mod expansion_parser;
pub mod lexer;
pub mod parser;
mod parser_compound;
mod parser_redirection;
pub mod parser_substitution;
mod parser_word;
pub mod types;
pub mod word_parser;

// Re-exports
pub use arithmetic_parser::{parse_arith_expr, parse_arithmetic_expression};
pub use arithmetic_primaries::parse_arith_number;
pub use lexer::{Lexer, LexerError, Token, TokenType};
pub use parser::{parse, Parser};
pub use types::ParseException;
