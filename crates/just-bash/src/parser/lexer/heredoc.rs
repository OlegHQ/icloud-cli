//! Heredoc lexing support

use super::token::{LexerError, Token, TokenType};
use super::Lexer;

/// Max heredoc size to prevent memory exhaustion (10MB)
const MAX_HEREDOC_SIZE: usize = 10_485_760;

/// Pending heredoc information
#[derive(Debug, Clone)]
pub(crate) struct PendingHeredoc {
    pub(crate) delimiter: String,
    pub(crate) strip_tabs: bool,
    pub(crate) quoted: bool,
}

impl Lexer {
    pub(crate) fn read_heredoc_content(&mut self) -> Result<(), LexerError> {
        while let Some(heredoc) = self.pending_heredocs.pop() {
            let start = self.pos;
            let start_line = self.line;
            let start_column = self.column;
            let mut content = String::new();

            while self.pos < self.input.len() {
                let mut line_content = String::new();

                // Read one line
                while self.pos < self.input.len() && self.input[self.pos] != '\n' {
                    line_content.push(self.input[self.pos]);
                    self.pos += 1;
                    self.column += 1;
                }

                // Check for delimiter
                let line_to_check = if heredoc.strip_tabs {
                    line_content.trim_start_matches('\t').to_string()
                } else {
                    line_content.clone()
                };

                if line_to_check == heredoc.delimiter {
                    // Consume the newline
                    if self.pos < self.input.len() && self.input[self.pos] == '\n' {
                        self.pos += 1;
                        self.line += 1;
                        self.column = 1;
                    }
                    break;
                }

                content.push_str(&line_content);
                // Check heredoc size limit to prevent memory exhaustion
                if content.len() > MAX_HEREDOC_SIZE {
                    return Err(LexerError::new(
                        format!("Heredoc size limit exceeded ({} bytes)", MAX_HEREDOC_SIZE),
                        start_line,
                        start_column,
                    ));
                }
                if self.pos < self.input.len() && self.input[self.pos] == '\n' {
                    content.push('\n');
                    self.pos += 1;
                    self.line += 1;
                    self.column = 1;
                }
            }

            self.tokens.push(Token::new(
                TokenType::HeredocContent,
                content,
                start,
                self.pos,
                start_line,
                start_column,
            ));
        }
        Ok(())
    }

    pub(crate) fn register_heredoc_from_lookahead(&mut self, strip_tabs: bool) {
        let saved_pos = self.pos;
        let saved_column = self.column;

        // Skip whitespace
        while self.pos < self.input.len() && matches!(self.input.get(self.pos), Some(' ' | '\t')) {
            self.pos += 1;
            self.column += 1;
        }

        let mut delimiter = String::new();
        let mut quoted = false;

        while self.pos < self.input.len() {
            let c = self.input[self.pos];

            if c.is_whitespace() || matches!(c, ';' | '<' | '>' | '&' | '|' | '(' | ')') {
                break;
            }

            if c == '\'' || c == '"' {
                quoted = true;
                let quote_char = c;
                self.pos += 1;
                self.column += 1;
                while self.pos < self.input.len() && self.input[self.pos] != quote_char {
                    delimiter.push(self.input[self.pos]);
                    self.pos += 1;
                    self.column += 1;
                }
                if self.pos < self.input.len() && self.input[self.pos] == quote_char {
                    self.pos += 1;
                    self.column += 1;
                }
            } else if c == '\\' {
                quoted = true;
                self.pos += 1;
                self.column += 1;
                if self.pos < self.input.len() {
                    delimiter.push(self.input[self.pos]);
                    self.pos += 1;
                    self.column += 1;
                }
            } else {
                delimiter.push(c);
                self.pos += 1;
                self.column += 1;
            }
        }

        self.pos = saved_pos;
        self.column = saved_column;

        if !delimiter.is_empty() {
            self.pending_heredocs.push(PendingHeredoc {
                delimiter,
                strip_tabs,
                quoted,
            });
        }
    }

    /// Add a pending heredoc (used by parser)
    pub fn add_pending_heredoc(&mut self, delimiter: String, strip_tabs: bool, quoted: bool) {
        self.pending_heredocs.push(PendingHeredoc {
            delimiter,
            strip_tabs,
            quoted,
        });
    }
}
