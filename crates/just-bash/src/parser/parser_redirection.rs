//! Redirection and simple command parsing for the Parser.
//!
//! Contains redirection recognition, redirection parsing, here-document handling,
//! optional redirection parsing, and simple command parsing.

use crate::ast::types::{CommandNode, RedirectionNode, AST};

use super::parser::Parser;
use crate::parser::lexer::TokenType;
use crate::parser::types::ParseException;

impl Parser {
    pub(super) fn is_redirection(&self) -> bool {
        let t = self.current().token_type;
        matches!(
            t,
            TokenType::Less
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

    pub(super) fn do_parse_redirection(
        &mut self,
    ) -> Result<Option<RedirectionNode>, ParseException> {
        let mut fd = None;
        let fd_variable: Option<String> = None;

        // Check for fd or variable before redirection operator
        if self.check(&[TokenType::Number]) {
            fd = Some(self.advance().value.parse::<i32>().unwrap_or(0));
        }

        let operator = if self.check(&[TokenType::Less]) {
            self.advance();
            crate::ast::types::RedirectionOperator::Less
        } else if self.check(&[TokenType::Great]) {
            self.advance();
            crate::ast::types::RedirectionOperator::Great
        } else if self.check(&[TokenType::DGreat]) {
            self.advance();
            crate::ast::types::RedirectionOperator::DGreat
        } else if self.check(&[TokenType::LessAnd]) {
            self.advance();
            crate::ast::types::RedirectionOperator::LessAnd
        } else if self.check(&[TokenType::GreatAnd]) {
            self.advance();
            crate::ast::types::RedirectionOperator::GreatAnd
        } else if self.check(&[TokenType::LessGreat]) {
            self.advance();
            crate::ast::types::RedirectionOperator::LessGreat
        } else if self.check(&[TokenType::DLessDash]) {
            self.advance();
            crate::ast::types::RedirectionOperator::DLessDash
        } else if self.check(&[TokenType::Clobber]) {
            self.advance();
            crate::ast::types::RedirectionOperator::Clobber
        } else if self.check(&[TokenType::TLess]) {
            self.advance();
            crate::ast::types::RedirectionOperator::TLess
        } else if self.check(&[TokenType::AndGreat]) {
            self.advance();
            crate::ast::types::RedirectionOperator::AndGreat
        } else if self.check(&[TokenType::AndDGreat]) {
            self.advance();
            crate::ast::types::RedirectionOperator::AndDGreat
        } else if self.check(&[TokenType::DLess]) {
            self.advance();
            crate::ast::types::RedirectionOperator::DLess
        } else {
            return Ok(None);
        };

        // Parse target
        let target = if matches!(
            operator,
            crate::ast::types::RedirectionOperator::DLess
                | crate::ast::types::RedirectionOperator::DLessDash
        ) {
            // Here-document
            let delimiter = if self.is_word() {
                self.advance().value
            } else {
                return Err(ParseException::new(
                    "Expected here-document delimiter",
                    self.current().line,
                    self.current().column,
                ));
            };

            let quoted = delimiter.starts_with('\'') || delimiter.starts_with('"');
            let strip_tabs = operator == crate::ast::types::RedirectionOperator::DLessDash;

            // Add to pending heredocs
            let redirect_idx = self.pending_redirections_len();
            self.add_pending_heredoc(redirect_idx, delimiter.clone(), strip_tabs, quoted);

            let redirection = AST::redirection(
                operator,
                crate::ast::types::RedirectionTarget::HereDoc(AST::here_doc(
                    &delimiter,
                    AST::word(vec![]),
                    strip_tabs,
                    quoted,
                )),
                fd,
                fd_variable.clone(),
            );
            self.push_pending_redirection(redirection.clone());

            crate::ast::types::RedirectionTarget::HereDoc(AST::here_doc(
                delimiter,
                AST::word(vec![]),
                strip_tabs,
                quoted,
            ))
        } else {
            // Word target
            let word = if self.is_word() {
                self.parse_word()?
            } else {
                return Err(ParseException::new(
                    "Expected redirection target",
                    self.current().line,
                    self.current().column,
                ));
            };
            crate::ast::types::RedirectionTarget::Word(word)
        };

        Ok(Some(AST::redirection(
            operator,
            target,
            fd,
            fd_variable.clone(),
        )))
    }

    pub fn parse_optional_redirections(&mut self) -> Result<Vec<RedirectionNode>, ParseException> {
        let mut redirections = Vec::new();

        while self.is_redirection() {
            self.check_iteration_limit()?;
            let pos_before = self.get_pos();

            if let Some(redir) = self.do_parse_redirection()? {
                redirections.push(redir);
            }

            if self.get_pos() == pos_before {
                break;
            }
        }

        Ok(redirections)
    }

    pub(super) fn parse_simple_command(&mut self) -> Result<CommandNode, ParseException> {
        let mut assignments = Vec::new();
        let mut name = None;
        let mut args = Vec::new();
        let mut redirections = Vec::new();

        // Parse assignments before command name
        while self.check(&[TokenType::AssignmentWord]) {
            let token = self.advance();
            let value = token.value.clone();
            // Parse VAR=value or VAR+=value
            if let Some(eq_pos) = value.find('=') {
                let var_name = &value[..eq_pos];
                let var_value = &value[eq_pos + 1..];
                let append = var_value.starts_with('+');
                let actual_value = if append { &var_value[1..] } else { var_value };

                assignments.push(crate::ast::types::AssignmentNode {
                    name: var_name.to_string(),
                    value: Some(self.parse_word_from_string(
                        actual_value,
                        false,
                        false,
                        true,
                        false,
                    )),
                    append,
                    array: None,
                });
            }
        }

        // Parse command name
        if self.is_word() {
            name = Some(self.parse_word()?);
        }

        // Parse arguments
        while self.is_word() {
            args.push(self.parse_word()?);
        }

        // Parse redirections
        while self.is_redirection() {
            if let Some(redir) = self.do_parse_redirection()? {
                redirections.push(redir);
            } else {
                break;
            }
        }

        Ok(CommandNode::Simple(AST::simple_command(
            name,
            args,
            assignments,
            redirections,
        )))
    }
}
