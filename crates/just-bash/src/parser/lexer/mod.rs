//! Lexer for Bash Scripts
//!
//! The lexer tokenizes input into a stream of tokens that the parser consumes.
//! It handles:
//! - Operators and delimiters
//! - Words (with quoting rules)
//! - Comments
//! - Here-documents
//! - Escape sequences

mod heredoc;
mod lookahead;
mod operators;
mod token;
mod word;

pub use token::{LexerError, Token, TokenType};

use heredoc::PendingHeredoc;
use operators::{is_word_boundary, SINGLE_CHAR_OPS, THREE_CHAR_OPS, TWO_CHAR_OPS};

/// Lexer class
pub struct Lexer {
    input: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
    tokens: Vec<Token>,
    pending_heredocs: Vec<PendingHeredoc>,
    /// Track depth inside (( )) for C-style for loops and arithmetic commands
    dparen_depth: usize,
}

impl Lexer {
    pub fn new(input: &str) -> Self {
        Self {
            input: input.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
            tokens: Vec::new(),
            pending_heredocs: Vec::new(),
            dparen_depth: 0,
        }
    }

    /// Tokenize the entire input
    pub fn tokenize(mut self) -> Result<Vec<Token>, LexerError> {
        let len = self.input.len();

        while self.pos < len {
            // Check for pending here-documents after newline
            if !self.pending_heredocs.is_empty()
                && !self.tokens.is_empty()
                && self.tokens.last().map(|t| t.token_type) == Some(TokenType::Newline)
            {
                self.read_heredoc_content()?;
                continue;
            }

            self.skip_whitespace();

            if self.pos >= len {
                break;
            }

            if let Some(token) = self.next_token()? {
                self.tokens.push(token);
            }
        }

        // Add EOF token
        self.tokens.push(Token::new(
            TokenType::Eof,
            "",
            self.pos,
            self.pos,
            self.line,
            self.column,
        ));

        Ok(self.tokens)
    }

    fn current(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.current();
        if self.pos < self.input.len() {
            self.pos += 1;
            self.column += 1;
        }
        c
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.current() {
            match c {
                ' ' | '\t' => {
                    self.pos += 1;
                    self.column += 1;
                }
                '\\' if self.peek(1) == Some('\n') => {
                    // Line continuation
                    self.pos += 2;
                    self.line += 1;
                    self.column = 1;
                }
                _ => break,
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, LexerError> {
        let start_line = self.line;
        let start_column = self.column;
        let start_pos = self.pos;

        let c0 = match self.current() {
            Some(c) => c,
            None => return Ok(None),
        };
        let c1 = self.peek(1);
        let c2 = self.peek(2);

        // Comments - but NOT inside (( )) arithmetic context where # is part of base notation
        if c0 == '#' && self.dparen_depth == 0 {
            return Ok(Some(self.read_comment(start_pos, start_line, start_column)));
        }

        // Newline
        if c0 == '\n' {
            self.pos += 1;
            self.line += 1;
            self.column = 1;
            return Ok(Some(Token::new(
                TokenType::Newline,
                "\n",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        // Three-character operators
        // Special case: <<- (heredoc with tab stripping)
        if c0 == '<' && c1 == Some('<') && c2 == Some('-') {
            self.pos += 3;
            self.column += 3;
            self.register_heredoc_from_lookahead(true);
            return Ok(Some(Token::new(
                TokenType::DLessDash,
                "<<-",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        // Table-driven three-char operators
        for (op_str, token_type) in THREE_CHAR_OPS {
            let chars: Vec<char> = op_str.chars().collect();
            if c0 == chars[0] && c1 == Some(chars[1]) && c2 == Some(chars[2]) {
                self.pos += 3;
                self.column += 3;
                return Ok(Some(Token::new(
                    *token_type,
                    *op_str,
                    start_pos,
                    self.pos,
                    start_line,
                    start_column,
                )));
            }
        }

        // Two-character operators
        // Special case: << (heredoc)
        if c0 == '<' && c1 == Some('<') {
            self.pos += 2;
            self.column += 2;
            self.register_heredoc_from_lookahead(false);
            return Ok(Some(Token::new(
                TokenType::DLess,
                "<<",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        // Special handling for (( and )) to track nested parentheses
        if c0 == '(' && c1 == Some('(') {
            if self.dparen_depth > 0 {
                // Inside arithmetic context, (( is just two open parens
                self.pos += 1;
                self.column += 1;
                self.dparen_depth += 1;
                return Ok(Some(Token::new(
                    TokenType::LParen,
                    "(",
                    start_pos,
                    self.pos,
                    start_line,
                    start_column,
                )));
            }
            // Check if this looks like nested subshells
            if self.looks_like_nested_subshells(self.pos + 2)
                || self.dparen_closes_with_spaced_parens(self.pos + 2)
            {
                self.pos += 1;
                self.column += 1;
                return Ok(Some(Token::new(
                    TokenType::LParen,
                    "(",
                    start_pos,
                    self.pos,
                    start_line,
                    start_column,
                )));
            }
            self.pos += 2;
            self.column += 2;
            self.dparen_depth = 1;
            return Ok(Some(Token::new(
                TokenType::DParenStart,
                "((",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        if c0 == ')' && c1 == Some(')') {
            if self.dparen_depth == 1 {
                self.pos += 2;
                self.column += 2;
                self.dparen_depth = 0;
                return Ok(Some(Token::new(
                    TokenType::DParenEnd,
                    "))",
                    start_pos,
                    self.pos,
                    start_line,
                    start_column,
                )));
            } else if self.dparen_depth > 1 {
                self.pos += 1;
                self.column += 1;
                self.dparen_depth -= 1;
                return Ok(Some(Token::new(
                    TokenType::RParen,
                    ")",
                    start_pos,
                    self.pos,
                    start_line,
                    start_column,
                )));
            }
            // dparen_depth == 0: emit single RPAREN
            self.pos += 1;
            self.column += 1;
            return Ok(Some(Token::new(
                TokenType::RParen,
                ")",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        // Table-driven two-char operators
        for (op_str, token_type) in TWO_CHAR_OPS {
            let chars: Vec<char> = op_str.chars().collect();
            if chars.len() >= 2 && c0 == chars[0] && c1 == Some(chars[1]) {
                // Skip (( and )) handled above
                if *op_str == "((" || *op_str == "))" {
                    continue;
                }
                // Skip ;; and ;;& inside (( )) context
                if self.dparen_depth > 0
                    && chars[0] == ';'
                    && (*token_type == TokenType::DSemi
                        || *token_type == TokenType::SemiAnd
                        || *token_type == TokenType::SemiSemiAnd)
                {
                    continue;
                }
                // Special case: [[ and ]] should only be recognized at word boundary
                if *token_type == TokenType::DBrackStart || *token_type == TokenType::DBrackEnd {
                    if let Some(after) = self.peek(2) {
                        if !is_word_boundary(after) {
                            break;
                        }
                    }
                }
                self.pos += 2;
                self.column += 2;
                return Ok(Some(Token::new(
                    *token_type,
                    *op_str,
                    start_pos,
                    self.pos,
                    start_line,
                    start_column,
                )));
            }
        }

        // Single-character operators
        if c0 == '(' && self.dparen_depth > 0 {
            self.pos += 1;
            self.column += 1;
            self.dparen_depth += 1;
            return Ok(Some(Token::new(
                TokenType::LParen,
                "(",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }
        if c0 == ')' && self.dparen_depth > 1 {
            self.pos += 1;
            self.column += 1;
            self.dparen_depth -= 1;
            return Ok(Some(Token::new(
                TokenType::RParen,
                ")",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        if let Some(&token_type) = SINGLE_CHAR_OPS.get(&c0) {
            self.pos += 1;
            self.column += 1;
            return Ok(Some(Token::new(
                token_type,
                c0.to_string(),
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        // Special cases: { } !
        if c0 == '{' {
            // Check for FD variable syntax
            if let Some(fd_var) = self.scan_fd_variable(start_pos) {
                self.pos = fd_var.end;
                self.column = start_column + (fd_var.end - start_pos);
                return Ok(Some(Token::new(
                    TokenType::FdVariable,
                    fd_var.varname,
                    start_pos,
                    self.pos,
                    start_line,
                    start_column,
                )));
            }
            // Check for {} as a word
            if c1 == Some('}') {
                self.pos += 2;
                self.column += 2;
                return Ok(Some(
                    Token::new(
                        TokenType::Word,
                        "{}",
                        start_pos,
                        self.pos,
                        start_line,
                        start_column,
                    )
                    .with_quotes(false, false),
                ));
            }
            // Check for brace expansion
            if self.scan_brace_expansion(start_pos).is_some() {
                return self.read_word_with_brace_expansion(start_pos, start_line, start_column);
            }
            // Check for literal brace word
            if self.scan_literal_brace_word(start_pos).is_some() {
                return self.read_word_with_brace_expansion(start_pos, start_line, start_column);
            }
            // { must be followed by whitespace to be a group start
            if let Some(next) = c1 {
                if next != ' ' && next != '\t' && next != '\n' {
                    return self.read_word(start_pos, start_line, start_column);
                }
            }
            self.pos += 1;
            self.column += 1;
            return Ok(Some(Token::new(
                TokenType::LBrace,
                "{",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        if c0 == '}' {
            if self.is_word_char_following(self.pos + 1) {
                return self.read_word(start_pos, start_line, start_column);
            }
            self.pos += 1;
            self.column += 1;
            return Ok(Some(Token::new(
                TokenType::RBrace,
                "}",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        if c0 == '!' {
            if c1 == Some('=') {
                self.pos += 2;
                self.column += 2;
                return Ok(Some(Token::new(
                    TokenType::Word,
                    "!=",
                    start_pos,
                    self.pos,
                    start_line,
                    start_column,
                )));
            }
            self.pos += 1;
            self.column += 1;
            return Ok(Some(Token::new(
                TokenType::Bang,
                "!",
                start_pos,
                self.pos,
                start_line,
                start_column,
            )));
        }

        // Words
        self.read_word(start_pos, start_line, start_column)
    }

    fn read_comment(&mut self, start: usize, line: usize, column: usize) -> Token {
        while let Some(c) = self.current() {
            if c == '\n' {
                break;
            }
            self.pos += 1;
            self.column += 1;
        }
        let value: String = self.input[start..self.pos].iter().collect();
        Token::new(TokenType::Comment, value, start, self.pos, line, column)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let lexer = Lexer::new("echo hello");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens.len(), 3); // echo, hello, EOF
        assert_eq!(tokens[0].token_type, TokenType::Name);
        assert_eq!(tokens[0].value, "echo");
        assert_eq!(tokens[1].token_type, TokenType::Name);
        assert_eq!(tokens[1].value, "hello");
    }

    #[test]
    fn test_pipeline() {
        let lexer = Lexer::new("cat file | grep pattern");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[2].token_type, TokenType::Pipe);
    }

    #[test]
    fn test_redirection() {
        let lexer = Lexer::new("echo hello > file.txt");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[2].token_type, TokenType::Great);
    }

    #[test]
    fn test_assignment() {
        let lexer = Lexer::new("VAR=value");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].token_type, TokenType::AssignmentWord);
        assert_eq!(tokens[0].value, "VAR=value");
    }

    #[test]
    fn test_double_quotes() {
        let lexer = Lexer::new("echo \"hello world\"");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[1].value, "hello world");
        assert!(tokens[1].quoted);
    }

    #[test]
    fn test_single_quotes() {
        let lexer = Lexer::new("echo 'hello world'");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[1].value, "hello world");
        assert!(tokens[1].quoted);
        assert!(tokens[1].single_quoted);
    }

    #[test]
    fn test_reserved_words() {
        let lexer = Lexer::new("if then else fi");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].token_type, TokenType::If);
        assert_eq!(tokens[1].token_type, TokenType::Then);
        assert_eq!(tokens[2].token_type, TokenType::Else);
        assert_eq!(tokens[3].token_type, TokenType::Fi);
    }

    #[test]
    fn test_heredoc() {
        let lexer = Lexer::new("cat <<EOF\nhello\nEOF\n");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[1].token_type, TokenType::DLess);
        // Find heredoc content token
        let heredoc_token = tokens
            .iter()
            .find(|t| t.token_type == TokenType::HeredocContent);
        assert!(heredoc_token.is_some());
        assert_eq!(heredoc_token.unwrap().value, "hello\n");
    }

    #[test]
    fn test_comment() {
        let lexer = Lexer::new("echo hello # this is a comment");
        let tokens = lexer.tokenize().unwrap();
        let comment = tokens.iter().find(|t| t.token_type == TokenType::Comment);
        assert!(comment.is_some());
    }

    #[test]
    fn test_arithmetic() {
        let lexer = Lexer::new("(( x + 1 ))");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].token_type, TokenType::DParenStart);
    }

    #[test]
    fn test_conditional() {
        let lexer = Lexer::new("[[ -f file ]]");
        let tokens = lexer.tokenize().unwrap();
        assert_eq!(tokens[0].token_type, TokenType::DBrackStart);
    }
}
