//! Parser — thin wrapper around brush-parser.

pub use brush_parser::ast;
pub use brush_parser::{ParseError, ParserOptions, SourceInfo};

/// Parse a bash script string into a brush-parser AST.
pub fn parse(input: &str) -> Result<ast::Program, String> {
    let tokens = brush_parser::tokenize_str(input)
        .map_err(|e| format!("{}", e))?;
    let program = brush_parser::parse_tokens(
        &tokens,
        &ParserOptions::default(),
        &SourceInfo::default(),
    )
    .map_err(|e| format!("{}", e))?;
    Ok(program)
}
