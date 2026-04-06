//! Recursive Descent Parser for Bash Scripts
//!
//! This parser consumes tokens from the lexer and produces an AST.
//! It follows the bash grammar structure for correctness.
//!
//! Grammar (simplified):
//!   script       ::= statement*
//!   statement    ::= pipeline ((&&|'||') pipeline)*  [&]
//!   pipeline     ::= [!] command (| command)*
//!   command      ::= simple_command | compound_command | function_def
//!   simple_cmd   ::= (assignment)* [word] (word)* (redirection)*
//!   compound_cmd ::= if | for | while | until | case | subshell | group | (( | [[

use crate::ast::types::{
    CommandNode, CompoundCommandNode, DeferredError, PipelineNode, RedirectionNode, ScriptNode,
    StatementNode, StatementOperator, AST,
};
use crate::parser::lexer::{Lexer, Token, TokenType};
use crate::parser::types::{ParseException, MAX_PARSER_DEPTH};

// Constants for limits
pub const MAX_INPUT_SIZE: usize = 10_000_000;
pub const MAX_TOKENS: usize = 100_000;
pub const MAX_PARSE_ITERATIONS: usize = 1_000_000;
// Note: MAX_PARSER_DEPTH is imported from types.rs

/// Pending heredoc information
#[derive(Debug, Clone)]
pub(super) struct PendingHeredoc {
    pub(super) redirect_idx: usize,
    pub(super) delimiter: String,
    pub(super) strip_tabs: bool,
    pub(super) quoted: bool,
}

/// Guard that decrements parse depth when dropped
pub struct DepthGuard {
    depth: *mut usize,
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        unsafe {
            *self.depth -= 1;
        }
    }
}

/// Main parser struct
pub struct Parser {
    pub(super) tokens: Vec<Token>,
    pub(super) pos: usize,
    pending_heredocs: Vec<PendingHeredoc>,
    pub(super) pending_redirections: Vec<RedirectionNode>,
    parse_iterations: usize,
    parse_depth: usize,
    input: String,
}

impl Parser {
    /// Create a new parser instance
    pub fn new() -> Self {
        Parser {
            tokens: Vec::new(),
            pos: 0,
            pending_heredocs: Vec::new(),
            pending_redirections: Vec::new(),
            parse_iterations: 0,
            parse_depth: 0,
            input: String::new(),
        }
    }

    /// Get the raw input string being parsed.
    pub fn get_input(&self) -> &str {
        &self.input
    }

    /// Check parse iteration limit to prevent infinite loops
    pub fn check_iteration_limit(&mut self) -> Result<(), ParseException> {
        self.parse_iterations += 1;
        if self.parse_iterations > MAX_PARSE_ITERATIONS {
            return Err(ParseException::new(
                "Maximum parse iterations exceeded (possible infinite loop)",
                self.current().line,
                self.current().column,
            ));
        }
        Ok(())
    }

    /// Increment parse depth and check limit to prevent stack overflow
    /// from deeply nested constructs. Returns a guard that decrements depth when dropped.
    pub fn enter_depth(&mut self) -> Result<DepthGuard, ParseException> {
        self.parse_depth += 1;
        if self.parse_depth > MAX_PARSER_DEPTH {
            return Err(ParseException::new(
                &format!(
                    "Maximum parser nesting depth exceeded ({})",
                    MAX_PARSER_DEPTH
                ),
                self.current().line,
                self.current().column,
            ));
        }
        Ok(DepthGuard {
            depth: &mut self.parse_depth as *mut usize,
        })
    }

    /// Parse a bash script string
    pub fn parse(&mut self, input: &str) -> Result<ScriptNode, ParseException> {
        // Check input size limit
        if input.len() > MAX_INPUT_SIZE {
            return Err(ParseException::new(
                &format!(
                    "Input too large: {} bytes exceeds limit of {}",
                    input.len(),
                    MAX_INPUT_SIZE
                ),
                1,
                1,
            ));
        }

        self.input = input.to_string();
        let mut lexer = Lexer::new(input);
        self.tokens = lexer
            .tokenize()
            .map_err(|e| ParseException::new(&e.message, e.line, e.column))?;

        // Check token count limit
        if self.tokens.len() > MAX_TOKENS {
            return Err(ParseException::new(
                &format!(
                    "Too many tokens: {} exceeds limit of {}",
                    self.tokens.len(),
                    MAX_TOKENS
                ),
                1,
                1,
            ));
        }

        self.pos = 0;
        self.pending_heredocs = Vec::new();
        self.pending_redirections = Vec::new();
        self.parse_iterations = 0;
        self.parse_depth = 0;

        self.parse_script()
    }

    /// Parse from pre-tokenized input
    pub fn parse_tokens(&mut self, tokens: Vec<Token>) -> Result<ScriptNode, ParseException> {
        self.tokens = tokens;
        self.pos = 0;
        self.pending_heredocs = Vec::new();
        self.pending_redirections = Vec::new();
        self.parse_iterations = 0;
        self.parse_depth = 0;

        self.parse_script()
    }

    // ===========================================================================
    // HELPER METHODS
    // ===========================================================================

    pub(super) fn current(&self) -> Token {
        if self.pos < self.tokens.len() {
            self.tokens[self.pos].clone()
        } else if !self.tokens.is_empty() {
            self.tokens[self.tokens.len() - 1].clone()
        } else {
            Token {
                token_type: TokenType::Eof,
                value: String::new(),
                line: 1,
                column: 1,
                start: 0,
                end: 0,
                quoted: false,
                single_quoted: false,
            }
        }
    }

    pub(super) fn peek(&self, offset: usize) -> Token {
        let idx = self.pos + offset;
        if idx < self.tokens.len() {
            self.tokens[idx].clone()
        } else if !self.tokens.is_empty() {
            self.tokens[self.tokens.len() - 1].clone()
        } else {
            Token {
                token_type: TokenType::Eof,
                value: String::new(),
                line: 1,
                column: 1,
                start: 0,
                end: 0,
                quoted: false,
                single_quoted: false,
            }
        }
    }

    pub(super) fn advance(&mut self) -> Token {
        let token = self.current();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        token
    }

    pub(super) fn get_pos(&self) -> usize {
        self.pos
    }

    pub(super) fn check(&self, types: &[TokenType]) -> bool {
        let current_type = self.tokens.get(self.pos).map(|t| &t.token_type);
        types.iter().any(|t| current_type == Some(t))
    }

    pub(super) fn expect(
        &mut self,
        token_type: TokenType,
        message: Option<&str>,
    ) -> Result<Token, ParseException> {
        if self.check(&[token_type]) {
            Ok(self.advance())
        } else {
            let token = self.current();
            Err(ParseException::new(
                message.unwrap_or(&format!(
                    "Expected {:?}, got {:?}",
                    token_type, token.token_type
                )),
                token.line,
                token.column,
            ))
        }
    }

    #[allow(dead_code)]
    pub(super) fn error(&self, message: &str) -> Result<(), ParseException> {
        let token = self.current();
        Err(ParseException::new(message, token.line, token.column))
    }

    pub(super) fn skip_newlines(&mut self) {
        while self.check(&[TokenType::Newline, TokenType::Comment]) {
            if self.check(&[TokenType::Newline]) {
                self.advance();
                self.process_heredocs();
            } else {
                self.advance();
            }
        }
    }

    pub(super) fn skip_separators(&mut self, include_case_terminators: bool) {
        loop {
            if self.check(&[TokenType::Newline]) {
                self.advance();
                self.process_heredocs();
                continue;
            }
            if self.check(&[TokenType::Semicolon, TokenType::Comment]) {
                self.advance();
                continue;
            }
            if include_case_terminators
                && self.check(&[TokenType::DSemi, TokenType::SemiAnd, TokenType::SemiSemiAnd])
            {
                self.advance();
                continue;
            }
            break;
        }
    }

    pub(super) fn add_pending_heredoc(
        &mut self,
        redirect_idx: usize,
        delimiter: String,
        strip_tabs: bool,
        quoted: bool,
    ) {
        self.pending_heredocs.push(PendingHeredoc {
            redirect_idx,
            delimiter,
            strip_tabs,
            quoted,
        });
    }

    fn process_heredocs(&mut self) {
        // Process pending here-documents
        let heredocs = std::mem::take(&mut self.pending_heredocs);
        for heredoc in heredocs {
            if self.check(&[TokenType::HeredocContent]) {
                let content = self.advance();
                let content_word = if heredoc.quoted {
                    AST::word(vec![AST::literal(&content.value)])
                } else {
                    self.parse_word_from_string(&content.value, false, false, false, true)
                };

                if let Some(redirection) = self.pending_redirections.get_mut(heredoc.redirect_idx) {
                    redirection.target =
                        crate::ast::types::RedirectionTarget::HereDoc(AST::here_doc(
                            &heredoc.delimiter,
                            content_word,
                            heredoc.strip_tabs,
                            heredoc.quoted,
                        ));
                }
            }
        }
    }

    pub(super) fn pending_redirections_len(&self) -> usize {
        self.pending_redirections.len()
    }

    pub(super) fn push_pending_redirection(&mut self, redirection: RedirectionNode) {
        self.pending_redirections.push(redirection);
    }

    #[allow(dead_code)]
    fn is_statement_end(&self) -> bool {
        self.check(&[
            TokenType::Eof,
            TokenType::Newline,
            TokenType::Semicolon,
            TokenType::Amp,
            TokenType::AndAnd,
            TokenType::OrOr,
            TokenType::RParen,
            TokenType::RBrace,
            TokenType::DSemi,
            TokenType::SemiAnd,
            TokenType::SemiSemiAnd,
        ])
    }

    fn is_command_start(&self) -> bool {
        let t = self.current().token_type;
        matches!(
            t,
            TokenType::Word
                | TokenType::Name
                | TokenType::Number
                | TokenType::AssignmentWord
                | TokenType::If
                | TokenType::For
                | TokenType::While
                | TokenType::Until
                | TokenType::Case
                | TokenType::LParen
                | TokenType::LBrace
                | TokenType::DParenStart
                | TokenType::DBrackStart
                | TokenType::Function
                | TokenType::Bang
                | TokenType::Time
                | TokenType::In
                | TokenType::Less
                | TokenType::Great
                | TokenType::DLess
                | TokenType::DGreat
                | TokenType::LessAnd
                | TokenType::GreatAnd
                | TokenType::LessGreat
                | TokenType::DLessDash
                | TokenType::Clobber
                | TokenType::TLess
                | TokenType::AndGreat
                | TokenType::AndDGreat
        )
    }

    // ===========================================================================
    // SCRIPT PARSING
    // ===========================================================================

    fn parse_script(&mut self) -> Result<ScriptNode, ParseException> {
        let mut statements = Vec::new();
        let max_iterations = 10000;
        let mut iterations = 0;

        self.skip_newlines();

        while !self.check(&[TokenType::Eof]) {
            iterations += 1;
            if iterations > max_iterations {
                return Err(ParseException::new(
                    &format!("Parser stuck: too many iterations (>{}))", max_iterations),
                    self.current().line,
                    self.current().column,
                ));
            }

            // Check for unexpected tokens at statement start
            if let Some(deferred_error_stmt) = self.check_unexpected_token()? {
                statements.push(deferred_error_stmt);
                self.skip_separators(false);
                continue;
            }

            let pos_before = self.pos;
            if let Some(stmt) = self.parse_statement()? {
                statements.push(stmt);
            }
            // Don't skip case terminators at script level
            self.skip_separators(false);

            // Check for case terminators at script level - syntax errors
            if self.check(&[TokenType::DSemi, TokenType::SemiAnd, TokenType::SemiSemiAnd]) {
                return Err(ParseException::new(
                    &format!(
                        "syntax error near unexpected token `{}`",
                        self.current().value
                    ),
                    self.current().line,
                    self.current().column,
                ));
            }

            // Safety: if we didn't advance, force advance
            if self.pos == pos_before && !self.check(&[TokenType::Eof]) {
                self.advance();
            }
        }

        Ok(AST::script(statements))
    }

    fn check_unexpected_token(&mut self) -> Result<Option<StatementNode>, ParseException> {
        let t = self.current().token_type;
        let v = self.current().value.clone();

        // Check for unexpected reserved words
        if matches!(
            t,
            TokenType::Do
                | TokenType::Done
                | TokenType::Then
                | TokenType::Else
                | TokenType::Elif
                | TokenType::Fi
                | TokenType::Esac
        ) {
            return Err(ParseException::new(
                &format!("syntax error near unexpected token `{}`", v),
                self.current().line,
                self.current().column,
            ));
        }

        // Check for unexpected closing braces/parens - deferred errors
        if t == TokenType::RBrace || t == TokenType::RParen {
            let error_msg = format!("syntax error near unexpected token `{}`", v);
            self.advance(); // Consume the token
            return Ok(Some(AST::statement(
                vec![AST::pipeline(
                    vec![CommandNode::Simple(AST::simple_command(
                        None,
                        vec![],
                        vec![],
                        vec![],
                    ))],
                    false,
                    false,
                    false,
                    None,
                )],
                vec![],
                false,
                Some(DeferredError {
                    message: error_msg.clone(),
                    token: v,
                }),
                None,
            )));
        }

        // Check for case terminators
        if matches!(
            t,
            TokenType::DSemi | TokenType::SemiAnd | TokenType::SemiSemiAnd
        ) {
            return Err(ParseException::new(
                &format!("syntax error near unexpected token `{}`", v),
                self.current().line,
                self.current().column,
            ));
        }

        // Check for bare semicolon
        if t == TokenType::Semicolon {
            return Err(ParseException::new(
                &format!("syntax error near unexpected token `{}`", v),
                self.current().line,
                self.current().column,
            ));
        }

        // Check for pipe at statement start
        if t == TokenType::Pipe || t == TokenType::PipeAmp {
            return Err(ParseException::new(
                &format!("syntax error near unexpected token `{}`", v),
                self.current().line,
                self.current().column,
            ));
        }

        Ok(None)
    }

    // ===========================================================================
    // STATEMENT PARSING
    // ===========================================================================

    pub fn parse_statement(&mut self) -> Result<Option<StatementNode>, ParseException> {
        self.skip_newlines();

        if !self.is_command_start() {
            return Ok(None);
        }

        let start_offset = self.current().start;

        let mut pipelines = Vec::new();
        let mut operators = Vec::new();
        let mut background = false;

        // Parse first pipeline
        let first_pipeline = self.parse_pipeline()?;
        pipelines.push(first_pipeline);

        // Parse additional pipelines connected by && or ||
        while self.check(&[TokenType::AndAnd, TokenType::OrOr]) {
            let op = self.advance();
            operators.push(if op.token_type == TokenType::AndAnd {
                StatementOperator::And
            } else {
                StatementOperator::Or
            });
            self.skip_newlines();
            let next_pipeline = self.parse_pipeline()?;
            pipelines.push(next_pipeline);
        }

        // Check for background execution
        if self.check(&[TokenType::Amp]) {
            self.advance();
            background = true;
        }

        // Extract source text for verbose mode
        let end_offset = if self.pos > 0 && self.pos <= self.tokens.len() {
            self.tokens[self.pos - 1].end
        } else {
            start_offset
        };
        let source_text = self
            .input
            .get(start_offset..end_offset)
            .map(|s| s.to_string());

        Ok(Some(AST::statement(
            pipelines,
            operators,
            background,
            None,
            source_text,
        )))
    }

    // ===========================================================================
    // PIPELINE PARSING
    // ===========================================================================

    fn parse_pipeline(&mut self) -> Result<PipelineNode, ParseException> {
        // Check for 'time' keyword
        let mut timed = false;
        let mut time_posix = false;
        if self.check(&[TokenType::Time]) {
            self.advance();
            timed = true;
            // Check for -p option
            if self.check(&[TokenType::Word, TokenType::Name]) && self.current().value == "-p" {
                self.advance();
                time_posix = true;
            }
        }

        // Check for ! (negation)
        let mut negation_count = 0;
        while self.check(&[TokenType::Bang]) {
            self.advance();
            negation_count += 1;
        }
        let negated = negation_count % 2 == 1;

        let mut commands = Vec::new();
        let mut pipe_stderr = Vec::new();

        // Parse first command
        let first_cmd = self.parse_command()?;
        commands.push(first_cmd);

        // Parse additional commands in pipeline
        while self.check(&[TokenType::Pipe, TokenType::PipeAmp]) {
            let pipe_token = self.advance();
            self.skip_newlines();
            pipe_stderr.push(pipe_token.token_type == TokenType::PipeAmp);
            let next_cmd = self.parse_command()?;
            commands.push(next_cmd);
        }

        Ok(AST::pipeline(
            commands,
            negated,
            timed,
            time_posix,
            if pipe_stderr.is_empty() {
                None
            } else {
                Some(pipe_stderr)
            },
        ))
    }

    // ===========================================================================
    // COMMAND DISPATCH
    // ===========================================================================

    fn parse_command(&mut self) -> Result<CommandNode, ParseException> {
        // Check for compound commands
        if self.check(&[TokenType::If]) {
            return Ok(self.parse_if()?);
        }
        if self.check(&[TokenType::For]) {
            return Ok(self.parse_for()?);
        }
        if self.check(&[TokenType::While]) {
            return Ok(self.parse_while()?);
        }
        if self.check(&[TokenType::Until]) {
            return Ok(self.parse_until()?);
        }
        if self.check(&[TokenType::Case]) {
            return Ok(self.parse_case()?);
        }
        if self.check(&[TokenType::LParen]) {
            return Ok(self.parse_subshell()?);
        }
        if self.check(&[TokenType::LBrace]) {
            return Ok(self.parse_group()?);
        }
        if self.check(&[TokenType::DParenStart]) {
            if self.dparen_closes_with_spaced_parens() {
                return Ok(self.parse_nested_subshells_from_dparen()?);
            }
            return Ok(self.parse_arithmetic_command()?);
        }
        if self.check(&[TokenType::DBrackStart]) {
            return Ok(self.parse_conditional_command()?);
        }
        if self.check(&[TokenType::Function]) {
            return Ok(self.parse_function_def()?);
        }

        // Check for function definition: name () { ... }
        if self.check(&[TokenType::Name, TokenType::Word])
            && self.peek(1).token_type == TokenType::LParen
            && self.peek(2).token_type == TokenType::RParen
        {
            return Ok(self.parse_function_def()?);
        }

        // Simple command
        Ok(self.parse_simple_command()?)
    }

    fn dparen_closes_with_spaced_parens(&self) -> bool {
        let mut depth = 1;
        let mut offset = 1;

        while self.pos + offset < self.tokens.len() {
            let tok = self.peek(offset);
            if tok.token_type == TokenType::Eof {
                return false;
            }

            if matches!(tok.token_type, TokenType::DParenStart | TokenType::LParen) {
                depth += 1;
            } else if tok.token_type == TokenType::DParenEnd {
                depth -= 2;
                if depth <= 0 {
                    return false;
                }
            } else if tok.token_type == TokenType::RParen {
                depth -= 1;
                if depth == 0 {
                    let next_tok = self.peek(offset + 1);
                    if next_tok.token_type == TokenType::RParen {
                        return true;
                    }
                }
            }
            offset += 1;
        }

        false
    }

    // ===========================================================================
    // COMPOUND LIST PARSING
    // ===========================================================================

    pub fn parse_compound_list(&mut self) -> Result<Vec<StatementNode>, ParseException> {
        let _depth_guard = self.enter_depth()?;
        let mut statements = Vec::new();

        self.skip_newlines();

        while !self.check(&[
            TokenType::Eof,
            TokenType::Fi,
            TokenType::Else,
            TokenType::Elif,
            TokenType::Then,
            TokenType::Do,
            TokenType::Done,
            TokenType::Esac,
            TokenType::RParen,
            TokenType::RBrace,
            TokenType::DSemi,
            TokenType::SemiAnd,
            TokenType::SemiSemiAnd,
        ]) && self.is_command_start()
        {
            self.check_iteration_limit()?;
            let pos_before = self.pos;

            if let Some(stmt) = self.parse_statement()? {
                statements.push(stmt);
            }
            self.skip_separators(true);

            if self.pos == pos_before {
                break;
            }
        }

        Ok(statements)
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience function to parse a bash script
pub fn parse(input: &str) -> Result<ScriptNode, ParseException> {
    let mut parser = Parser::new();
    parser.parse(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty() {
        let mut parser = Parser::new();
        let result = parser.parse("");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 0);
    }

    #[test]
    fn test_parse_simple_command() {
        let mut parser = Parser::new();
        let result = parser.parse("echo hello");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_pipeline() {
        let mut parser = Parser::new();
        let result = parser.parse("echo hello | cat");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_function() {
        let mut parser = Parser::new();
        let result = parser.parse("foo() { echo bar; }");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_if_statement() {
        let mut parser = Parser::new();
        let result = parser.parse("if true; then echo yes; fi");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_for_loop() {
        let mut parser = Parser::new();
        let result = parser.parse("for i in a b c; do echo $i; done");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_while_loop() {
        let mut parser = Parser::new();
        let result = parser.parse("while true; do echo yes; done");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_case_statement() {
        let mut parser = Parser::new();
        let result = parser.parse("case $x in a) echo a;; esac");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_subshell() {
        let mut parser = Parser::new();
        let result = parser.parse("(echo hello)");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_group() {
        let mut parser = Parser::new();
        let result = parser.parse("{ echo hello; }");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }

    #[test]
    fn test_parse_arithmetic_command() {
        let mut parser = Parser::new();
        let result = parser.parse("((x = 1 + 2))");
        assert!(result.is_ok());
        let script = result.unwrap();
        assert_eq!(script.statements.len(), 1);
    }
}
