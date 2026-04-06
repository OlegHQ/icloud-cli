//! Compound command parsing for the Parser.
//!
//! Contains if/for/while/until/case/subshell/group commands,
//! function definitions, arithmetic commands, and conditional commands.

use crate::ast::types::{CommandNode, CompoundCommandNode, AST};
use crate::parser::conditional_parser::{
    parse_conditional_expression, CondParserContext, CondToken,
};
use crate::parser::word_parser::parse_arith_expr_from_string;
use std::cell::RefCell;

use super::parser::Parser;
use crate::parser::lexer::TokenType;
use crate::parser::types::ParseException;

impl Parser {
    pub(super) fn parse_if(&mut self) -> Result<CommandNode, ParseException> {
        self.expect(TokenType::If, None)?;

        let condition = self.parse_compound_list()?;

        self.expect(TokenType::Then, None)?;
        let mut then_body = Vec::new();
        while !self.check(&[
            TokenType::Eof,
            TokenType::Fi,
            TokenType::Elif,
            TokenType::Else,
        ]) {
            if let Some(stmt) = self.parse_statement()? {
                then_body.push(stmt);
            }
            self.skip_separators(true);
        }

        // Empty body is a syntax error in bash
        if then_body.is_empty() {
            let next_tok = if self.check(&[TokenType::Fi]) {
                "fi"
            } else if self.check(&[TokenType::Else]) {
                "else"
            } else if self.check(&[TokenType::Elif]) {
                "elif"
            } else {
                "fi"
            };
            return Err(ParseException::new(
                &format!("syntax error near unexpected token `{}'", next_tok),
                self.current().line,
                self.current().column,
            ));
        }

        let mut clauses = vec![crate::ast::types::IfClause {
            condition,
            body: then_body,
        }];

        // Parse elif clauses
        while self.check(&[TokenType::Elif]) {
            self.advance();
            let elif_condition = self.parse_compound_list()?;
            self.expect(TokenType::Then, None)?;
            let mut elif_body = Vec::new();
            while !self.check(&[
                TokenType::Eof,
                TokenType::Fi,
                TokenType::Elif,
                TokenType::Else,
            ]) {
                if let Some(stmt) = self.parse_statement()? {
                    elif_body.push(stmt);
                }
                self.skip_separators(true);
            }
            // Empty elif body is a syntax error
            if elif_body.is_empty() {
                let next_tok = if self.check(&[TokenType::Fi]) {
                    "fi"
                } else if self.check(&[TokenType::Else]) {
                    "else"
                } else if self.check(&[TokenType::Elif]) {
                    "elif"
                } else {
                    "fi"
                };
                return Err(ParseException::new(
                    &format!("syntax error near unexpected token `{}'", next_tok),
                    self.current().line,
                    self.current().column,
                ));
            }
            clauses.push(crate::ast::types::IfClause {
                condition: elif_condition,
                body: elif_body,
            });
        }

        // Parse else clause
        let mut else_body = None;
        if self.check(&[TokenType::Else]) {
            self.advance();
            let mut body = Vec::new();
            while !self.check(&[TokenType::Eof, TokenType::Fi]) {
                if let Some(stmt) = self.parse_statement()? {
                    body.push(stmt);
                }
                self.skip_separators(true);
            }
            // Empty else body is a syntax error
            if body.is_empty() {
                return Err(ParseException::new(
                    "syntax error near unexpected token `fi'",
                    self.current().line,
                    self.current().column,
                ));
            }
            else_body = Some(body);
        }

        self.expect(TokenType::Fi, None)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(CompoundCommandNode::If(
            AST::if_node(clauses, else_body, redirections),
        )))
    }

    pub(super) fn parse_for(&mut self) -> Result<CommandNode, ParseException> {
        self.expect(TokenType::For, None)?;

        // Check for C-style for: for (( ... ))
        if self.check(&[TokenType::DParenStart]) {
            return self.parse_c_style_for(String::new());
        }

        // Regular for: for VAR in WORDS
        let variable = if self.is_word() {
            self.advance().value
        } else {
            return Err(ParseException::new(
                "Expected variable name in for loop",
                self.current().line,
                self.current().column,
            ));
        };

        self.skip_newlines();

        // Check for "in" keyword
        let mut words = None;
        if self.check(&[TokenType::In]) {
            self.advance();
            let mut word_list = Vec::new();
            while !self.check(&[
                TokenType::Eof,
                TokenType::Newline,
                TokenType::Semicolon,
                TokenType::Do,
            ]) {
                if self.is_word() {
                    word_list.push(self.parse_word()?);
                } else {
                    break;
                }
            }
            words = Some(word_list);
        }

        self.skip_separators(false);
        self.expect(TokenType::Do, None)?;

        let body = self.parse_compound_list()?;

        self.expect(TokenType::Done, None)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(CompoundCommandNode::For(
            AST::for_node(variable, words, body, redirections),
        )))
    }

    fn parse_c_style_for(&mut self, _variable: String) -> Result<CommandNode, ParseException> {
        self.expect(TokenType::DParenStart, None)?;

        // Parse init; cond; step
        let mut init_str = String::new();
        let mut cond_str = String::new();
        let mut step_str = String::new();
        let mut current_phase = 0; // 0=init, 1=cond, 2=step
        let mut paren_depth = 0;
        let mut dparen_depth = 1;

        while dparen_depth > 0 && !self.check(&[TokenType::Eof]) {
            if self.check(&[TokenType::Semicolon]) {
                current_phase += 1;
                self.advance();
                continue;
            }

            let current_str = match current_phase {
                0 => &mut init_str,
                1 => &mut cond_str,
                _ => &mut step_str,
            };

            if self.check(&[TokenType::DParenStart]) {
                dparen_depth += 1;
                if !current_str.is_empty() {
                    current_str.push_str("((");
                }
                self.advance();
            } else if self.check(&[TokenType::DParenEnd]) {
                dparen_depth -= 1;
                if dparen_depth > 0 && !current_str.is_empty() {
                    current_str.push_str("))");
                }
                self.advance();
                if dparen_depth == 0 {
                    break;
                }
            } else if self.check(&[TokenType::LParen]) {
                paren_depth += 1;
                current_str.push('(');
                self.advance();
            } else if self.check(&[TokenType::RParen]) {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                current_str.push(')');
                self.advance();
            } else {
                let val = &self.current().value;
                if !current_str.is_empty() && !current_str.ends_with(' ') {
                    current_str.push(' ');
                }
                current_str.push_str(val);
                self.advance();
            }
        }

        self.expect(TokenType::DParenEnd, None)?;
        self.skip_newlines();
        if self.check(&[TokenType::Semicolon]) {
            self.advance();
        }
        self.skip_newlines();

        // Accept either do...done or { } for body (bash allows both)
        let body = if self.check(&[TokenType::LBrace]) {
            self.advance();
            let body = self.parse_compound_list()?;
            self.expect(TokenType::RBrace, None)?;
            body
        } else {
            self.expect(TokenType::Do, None)?;
            let body = self.parse_compound_list()?;
            self.expect(TokenType::Done, None)?;
            body
        };

        let redirections = self.parse_optional_redirections()?;

        let init = if init_str.trim().is_empty() {
            None
        } else {
            Some(parse_arith_expr_from_string(init_str.trim()))
        };
        let condition = if cond_str.trim().is_empty() {
            None
        } else {
            Some(parse_arith_expr_from_string(cond_str.trim()))
        };
        let update = if step_str.trim().is_empty() {
            None
        } else {
            Some(parse_arith_expr_from_string(step_str.trim()))
        };

        Ok(CommandNode::Compound(CompoundCommandNode::CStyleFor(
            crate::ast::types::CStyleForNode {
                init,
                condition,
                update,
                body,
                redirections,
                line: None,
            },
        )))
    }

    pub(super) fn parse_while(&mut self) -> Result<CommandNode, ParseException> {
        self.expect(TokenType::While, None)?;

        let condition = self.parse_compound_list()?;

        self.skip_separators(false);
        self.expect(TokenType::Do, None)?;

        let body = self.parse_compound_list()?;

        // Empty body is a syntax error in bash
        if body.is_empty() {
            return Err(ParseException::new(
                "syntax error near unexpected token `done'",
                self.current().line,
                self.current().column,
            ));
        }

        self.expect(TokenType::Done, None)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(CompoundCommandNode::While(
            AST::while_node(condition, body, redirections),
        )))
    }

    pub(super) fn parse_until(&mut self) -> Result<CommandNode, ParseException> {
        self.expect(TokenType::Until, None)?;

        let condition = self.parse_compound_list()?;

        self.skip_separators(false);
        self.expect(TokenType::Do, None)?;

        let body = self.parse_compound_list()?;

        // Empty body is a syntax error in bash
        if body.is_empty() {
            return Err(ParseException::new(
                "syntax error near unexpected token `done'",
                self.current().line,
                self.current().column,
            ));
        }

        self.expect(TokenType::Done, None)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(CompoundCommandNode::Until(
            AST::until_node(condition, body, redirections),
        )))
    }

    pub(super) fn parse_case(&mut self) -> Result<CommandNode, ParseException> {
        self.expect(TokenType::Case, None)?;

        if !self.is_word() {
            return Err(ParseException::new(
                "Expected word after 'case'",
                self.current().line,
                self.current().column,
            ));
        }
        let word = self.parse_word()?;

        self.skip_newlines();
        self.expect(TokenType::In, None)?;
        self.skip_newlines();

        let mut items = Vec::new();

        while !self.check(&[TokenType::Eof, TokenType::Esac]) {
            self.check_iteration_limit()?;
            let pos_before = self.get_pos();

            if self.check(&[TokenType::Newline, TokenType::Semicolon]) {
                self.advance();
                continue;
            }
            if self.check(&[TokenType::Esac]) {
                break;
            }

            // Skip optional (
            if self.check(&[TokenType::LParen]) {
                self.advance();
            }

            // Parse patterns
            let mut patterns = Vec::new();
            while self.is_word() {
                patterns.push(self.parse_word()?);

                if self.check(&[TokenType::Pipe]) {
                    self.advance();
                } else {
                    break;
                }
            }

            if patterns.is_empty() {
                // Safety: if we didn't get any patterns and didn't advance, break
                if self.get_pos() == pos_before {
                    break;
                }
                continue;
            }

            self.expect(TokenType::RParen, None)?;
            self.skip_newlines();

            // Parse body
            let mut body = Vec::new();
            while !self.check(&[
                TokenType::Eof,
                TokenType::DSemi,
                TokenType::SemiAnd,
                TokenType::SemiSemiAnd,
                TokenType::Esac,
            ]) {
                self.check_iteration_limit()?;

                // Check if we're looking at the start of another case pattern (word followed by ))
                if self.is_word() && self.peek(1).token_type == TokenType::RParen {
                    return Err(ParseException::new(
                        "syntax error near unexpected token `)'",
                        self.current().line,
                        self.current().column,
                    ));
                }
                // Also check for optional ( before pattern
                if self.check(&[TokenType::LParen]) && self.peek(1).token_type == TokenType::Word {
                    let next_val = self.peek(1).value.clone();
                    return Err(ParseException::new(
                        &format!("syntax error near unexpected token `{}'", next_val),
                        self.current().line,
                        self.current().column,
                    ));
                }

                let inner_pos_before = self.get_pos();
                if let Some(stmt) = self.parse_statement()? {
                    body.push(stmt);
                }
                // Don't skip case terminators (;;, ;&, ;;&) - we need to see them
                self.skip_separators(false);

                // If we didn't advance and didn't get a statement, break to avoid infinite loop
                if self.get_pos() == inner_pos_before {
                    break;
                }
            }

            // Parse terminator
            let terminator = if self.check(&[TokenType::DSemi]) {
                self.advance();
                crate::ast::types::CaseTerminator::DoubleSemi
            } else if self.check(&[TokenType::SemiAnd]) {
                self.advance();
                crate::ast::types::CaseTerminator::SemiAnd
            } else if self.check(&[TokenType::SemiSemiAnd]) {
                self.advance();
                crate::ast::types::CaseTerminator::SemiSemiAnd
            } else {
                crate::ast::types::CaseTerminator::DoubleSemi
            };

            items.push(crate::ast::types::CaseItemNode {
                patterns,
                body,
                terminator,
            });

            self.skip_newlines();

            // Safety: if we didn't advance and didn't get an item, break
            if self.get_pos() == pos_before {
                break;
            }
        }

        self.expect(TokenType::Esac, None)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(CompoundCommandNode::Case(
            AST::case_node(word, items, redirections),
        )))
    }

    pub(super) fn parse_subshell(&mut self) -> Result<CommandNode, ParseException> {
        self.expect(TokenType::LParen, None)?;

        let body = self.parse_compound_list()?;

        self.expect(TokenType::RParen, None)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(CompoundCommandNode::Subshell(
            AST::subshell(body, redirections),
        )))
    }

    pub(super) fn parse_group(&mut self) -> Result<CommandNode, ParseException> {
        self.expect(TokenType::LBrace, None)?;

        let body = self.parse_compound_list()?;

        self.expect(TokenType::RBrace, None)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(CompoundCommandNode::Group(
            AST::group(body, redirections),
        )))
    }

    pub(super) fn parse_function_def(&mut self) -> Result<CommandNode, ParseException> {
        let name: String;

        if self.check(&[TokenType::Function]) {
            self.advance();
            if self.check(&[TokenType::Name, TokenType::Word]) {
                name = self.advance().value;
            } else {
                return Err(ParseException::new(
                    "Expected function name",
                    self.current().line,
                    self.current().column,
                ));
            }

            // Optional ()
            if self.check(&[TokenType::LParen]) {
                self.advance();
                self.expect(TokenType::RParen, None)?;
            }
        } else {
            name = self.advance().value;
            if name.contains('$') {
                return Err(ParseException::new(
                    &format!("`{}': not a valid identifier", name),
                    self.current().line,
                    self.current().column,
                ));
            }
            self.expect(TokenType::LParen, None)?;
            self.expect(TokenType::RParen, None)?;
        }

        self.skip_newlines();

        let body = self.parse_compound_command_body(true)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::FunctionDef(AST::function_def(
            name,
            body,
            redirections,
            None,
        )))
    }

    fn parse_compound_command_body(
        &mut self,
        _for_function_body: bool,
    ) -> Result<CompoundCommandNode, ParseException> {
        let cmd = if self.check(&[TokenType::LBrace]) {
            self.parse_group()?
        } else if self.check(&[TokenType::LParen]) {
            self.parse_subshell()?
        } else if self.check(&[TokenType::If]) {
            self.parse_if()?
        } else if self.check(&[TokenType::For]) {
            self.parse_for()?
        } else if self.check(&[TokenType::While]) {
            self.parse_while()?
        } else if self.check(&[TokenType::Until]) {
            self.parse_until()?
        } else if self.check(&[TokenType::Case]) {
            self.parse_case()?
        } else {
            return Err(ParseException::new(
                "Expected compound command for function body",
                self.current().line,
                self.current().column,
            ));
        };

        match cmd {
            CommandNode::Compound(compound) => Ok(compound),
            _ => Err(ParseException::new(
                "Expected compound command for function body",
                self.current().line,
                self.current().column,
            )),
        }
    }

    pub(super) fn parse_arithmetic_command(&mut self) -> Result<CommandNode, ParseException> {
        let start_token = self.expect(TokenType::DParenStart, None)?;

        let mut expr_str = String::new();
        let mut dparen_depth = 1;
        let mut paren_depth = 0;
        let mut pending_rparen = false;
        let mut found_closing = false;

        while dparen_depth > 0 && !self.check(&[TokenType::Eof]) {
            if pending_rparen {
                pending_rparen = false;
                if paren_depth > 0 {
                    paren_depth -= 1;
                    expr_str.push(')');
                    continue;
                }
                if self.check(&[TokenType::RParen]) {
                    dparen_depth -= 1;
                    found_closing = true;
                    self.advance();
                    continue;
                }
                if self.check(&[TokenType::DParenEnd]) {
                    dparen_depth -= 1;
                    found_closing = true;
                    continue;
                }
                expr_str.push(')');
                continue;
            }

            if self.check(&[TokenType::DParenStart]) {
                dparen_depth += 1;
                expr_str.push_str("((");
                self.advance();
            } else if self.check(&[TokenType::DParenEnd]) {
                if paren_depth >= 2 {
                    paren_depth -= 2;
                    expr_str.push_str("))");
                    self.advance();
                } else if paren_depth == 1 {
                    paren_depth -= 1;
                    expr_str.push(')');
                    pending_rparen = true;
                    self.advance();
                } else {
                    dparen_depth -= 1;
                    found_closing = true;
                    if dparen_depth > 0 {
                        expr_str.push_str("))");
                    }
                    self.advance();
                }
            } else if self.check(&[TokenType::LParen]) {
                paren_depth += 1;
                expr_str.push('(');
                self.advance();
            } else if self.check(&[TokenType::RParen]) {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                expr_str.push(')');
                self.advance();
            } else {
                let token_value = self.current().value.clone();
                let last_char = expr_str.chars().last().unwrap_or(' ');

                let needs_space = !expr_str.is_empty()
                    && !expr_str.ends_with(' ')
                    && !(token_value == "="
                        && expr_str.ends_with(|c: char| "|&^+*-*/%<>".contains(c)))
                    && !(token_value == "<" && last_char == '<')
                    && !(token_value == ">" && last_char == '>');

                if needs_space {
                    expr_str.push(' ');
                }
                expr_str.push_str(&token_value);
                self.advance();
            }
        }

        if !found_closing {
            self.expect(TokenType::DParenEnd, None)?;
        }

        let expression = parse_arith_expr_from_string(expr_str.trim());
        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(
            CompoundCommandNode::ArithmeticCommand(AST::arithmetic_command(
                expression,
                redirections,
                Some(start_token.line),
            )),
        ))
    }

    pub(super) fn parse_conditional_command(&mut self) -> Result<CommandNode, ParseException> {
        let start_token = self.expect(TokenType::DBrackStart, None)?;

        let expression = self.do_parse_conditional_expression()?;

        self.expect(TokenType::DBrackEnd, None)?;

        let redirections = self.parse_optional_redirections()?;

        Ok(CommandNode::Compound(
            CompoundCommandNode::ConditionalCommand(AST::conditional_command(
                expression,
                redirections,
                Some(start_token.line),
            )),
        ))
    }

    fn do_parse_conditional_expression(
        &mut self,
    ) -> Result<crate::ast::types::ConditionalExpressionNode, ParseException> {
        // Use RefCell to allow multiple borrows in closures
        let parser = RefCell::new(self);

        let is_word = || parser.borrow().is_word();

        let check = |tt: TokenType| parser.borrow().check(&[tt]);

        let peek = |offset: isize| {
            let p = parser.borrow();
            let idx = if offset >= 0 {
                p.pos.saturating_add(offset as usize)
            } else {
                p.pos.saturating_sub((-offset) as usize)
            };
            if idx < p.tokens.len() {
                let t = &p.tokens[idx];
                CondToken {
                    token_type: t.token_type,
                    value: t.value.clone(),
                    quoted: t.quoted,
                    start: t.start,
                    end: t.end,
                }
            } else {
                CondToken {
                    token_type: TokenType::Eof,
                    value: String::new(),
                    quoted: false,
                    start: 0,
                    end: 0,
                }
            }
        };

        let current = || {
            let p = parser.borrow();
            let t = p.current();
            CondToken {
                token_type: t.token_type,
                value: t.value.clone(),
                quoted: t.quoted,
                start: t.start,
                end: t.end,
            }
        };

        let advance = || {
            let mut p = parser.borrow_mut();
            let t = p.advance();
            CondToken {
                token_type: t.token_type,
                value: t.value.clone(),
                quoted: t.quoted,
                start: t.start,
                end: t.end,
            }
        };

        let expect = |tt: TokenType| {
            let mut p = parser.borrow_mut();
            let _ = p.expect(tt, None);
        };

        let skip_newlines = || {
            parser.borrow_mut().skip_newlines();
        };

        let parse_word_no_brace_expansion = || {
            parser
                .borrow_mut()
                .parse_word_no_brace_expansion()
                .unwrap_or_else(|_| AST::word(vec![]))
        };

        let parse_word_for_regex = || {
            parser
                .borrow_mut()
                .parse_word_for_regex()
                .unwrap_or_else(|_| AST::word(vec![]))
        };

        let parse_word_from_string_fn =
            |value: &str,
             quoted: bool,
             single_quoted: bool,
             is_assignment: bool,
             here_doc: bool,
             no_brace_expansion: bool| {
                parser.borrow_mut().parse_word_from_string_full(
                    value,
                    quoted,
                    single_quoted,
                    is_assignment,
                    here_doc,
                    no_brace_expansion,
                    false,
                )
            };

        let get_input = || parser.borrow().get_input().to_string();

        let error = |msg: &str| {
            eprintln!("Conditional parse error: {}", msg);
        };

        let ctx = CondParserContext {
            is_word: &is_word,
            check: &check,
            peek: &peek,
            current: &current,
            advance: &advance,
            expect: &expect,
            skip_newlines: &skip_newlines,
            parse_word_no_brace_expansion: &parse_word_no_brace_expansion,
            parse_word_for_regex: &parse_word_for_regex,
            parse_word_from_string: &parse_word_from_string_fn,
            get_input: &get_input,
            error: &error,
        };

        Ok(parse_conditional_expression(&ctx))
    }

    pub(super) fn parse_nested_subshells_from_dparen(
        &mut self,
    ) -> Result<CommandNode, ParseException> {
        self.advance(); // Skip DPAREN_START

        let inner_body = self.parse_compound_list()?;

        self.expect(TokenType::RParen, None)?;
        self.expect(TokenType::RParen, None)?;

        let redirections = self.parse_optional_redirections()?;

        let inner_subshell = AST::subshell(inner_body, vec![]);

        Ok(CommandNode::Compound(CompoundCommandNode::Subshell(
            AST::subshell(
                vec![AST::statement(
                    vec![AST::pipeline(
                        vec![CommandNode::Compound(CompoundCommandNode::Subshell(
                            inner_subshell,
                        ))],
                        false,
                        false,
                        false,
                        None,
                    )],
                    vec![],
                    false,
                    None,
                    None,
                )],
                redirections,
            ),
        )))
    }
}
