//! Word token lexing
//!
//! Handles reading word tokens including quoting, expansions ($(...), ${...}, $[...]),
//! backtick substitution, escapes, brace expansion, and token classification
//! (reserved words, assignments, numbers, names).

use super::operators::{find_assignment_eq, is_valid_assignment_lhs, is_valid_name, is_word_boundary, RESERVED_WORDS};
use super::token::{LexerError, Token, TokenType};
use super::Lexer;

impl Lexer {
    pub(crate) fn read_word(
        &mut self,
        start: usize,
        line: usize,
        column: usize,
    ) -> Result<Option<Token>, LexerError> {
        let mut value = String::new();
        let mut quoted = false;
        let mut single_quoted = false;
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let starts_with_quote = matches!(self.current(), Some('"' | '\''));
        let mut has_content_after_quote = false;
        let mut bracket_depth = 0;
        let mut col = column;
        let mut ln = line;

        while let Some(c) = self.current() {
            // Check for word boundaries
            if !in_single_quote && !in_double_quote {
                // Handle extglob pattern
                if c == '('
                    && !value.is_empty()
                    && "@*+?!".contains(value.chars().last().unwrap_or(' '))
                {
                    if let Some(result) = self.scan_extglob_pattern(self.pos) {
                        value.push_str(&result.content);
                        self.pos = result.end;
                        col += result.content.len();
                        continue;
                    }
                }

                // Handle array subscript brackets
                if c == '[' && bracket_depth == 0 && is_valid_name(&value) {
                    if let Some(after) = self.peek(1) {
                        if after == '^' || after == '!' {
                            value.push(c);
                            self.pos += 1;
                            col += 1;
                            continue;
                        }
                    }
                    bracket_depth = 1;
                    value.push(c);
                    self.pos += 1;
                    col += 1;
                    continue;
                } else if c == '[' && bracket_depth > 0 {
                    if !value.is_empty() && value.chars().last() != Some('\\') {
                        bracket_depth += 1;
                    }
                    value.push(c);
                    self.pos += 1;
                    col += 1;
                    continue;
                } else if c == ']' && bracket_depth > 0 {
                    if !value.is_empty() && value.chars().last() != Some('\\') {
                        bracket_depth -= 1;
                    }
                    value.push(c);
                    self.pos += 1;
                    col += 1;
                    continue;
                }

                // Inside brackets, only break on newlines
                if bracket_depth > 0 {
                    if c == '\n' {
                        break;
                    }
                    value.push(c);
                    self.pos += 1;
                    col += 1;
                    continue;
                }

                if is_word_boundary(c) {
                    break;
                }
            }

            // Handle $'' ANSI-C quoting
            if c == '$' && self.peek(1) == Some('\'') && !in_single_quote && !in_double_quote {
                value.push_str("$'");
                self.pos += 2;
                col += 2;
                while let Some(ch) = self.current() {
                    if ch == '\'' {
                        break;
                    }
                    if ch == '\\' && self.peek(1).is_some() {
                        value.push(ch);
                        value.push(self.peek(1).unwrap());
                        self.pos += 2;
                        col += 2;
                    } else {
                        value.push(ch);
                        self.pos += 1;
                        col += 1;
                    }
                }
                if self.current() == Some('\'') {
                    value.push('\'');
                    self.pos += 1;
                    col += 1;
                }
                continue;
            }

            // Handle $"..." locale quoting
            if c == '$' && self.peek(1) == Some('"') && !in_single_quote && !in_double_quote {
                self.pos += 1;
                col += 1;
                in_double_quote = true;
                quoted = true;
                if value.is_empty() {
                    // Treat as if word started with quote
                }
                self.pos += 1;
                col += 1;
                continue;
            }

            // Handle quotes
            if c == '\'' && !in_double_quote {
                if in_single_quote {
                    in_single_quote = false;
                    if !starts_with_quote || has_content_after_quote {
                        value.push(c);
                    } else if let Some(next) = self.peek(1) {
                        if !is_word_boundary(next) && next != '\'' {
                            if next == '"' {
                                has_content_after_quote = true;
                                value.push(c);
                                single_quoted = false;
                                quoted = false;
                            } else {
                                has_content_after_quote = true;
                                value.push(c);
                            }
                        }
                    }
                } else {
                    in_single_quote = true;
                    if starts_with_quote && !has_content_after_quote {
                        single_quoted = true;
                        quoted = true;
                    } else {
                        value.push(c);
                    }
                }
                self.pos += 1;
                col += 1;
                continue;
            }

            if c == '"' && !in_single_quote {
                if in_double_quote {
                    in_double_quote = false;
                    if !starts_with_quote || has_content_after_quote {
                        value.push(c);
                    } else if let Some(next) = self.peek(1) {
                        if !is_word_boundary(next) && next != '"' {
                            if next == '\'' {
                                has_content_after_quote = true;
                                value.push(c);
                                single_quoted = false;
                                quoted = false;
                            } else {
                                has_content_after_quote = true;
                                value.push(c);
                            }
                        }
                    }
                } else {
                    in_double_quote = true;
                    if starts_with_quote && !has_content_after_quote {
                        quoted = true;
                    } else {
                        value.push(c);
                    }
                }
                self.pos += 1;
                col += 1;
                continue;
            }

            // Handle escapes
            if c == '\\' && !in_single_quote {
                if let Some(next) = self.peek(1) {
                    if next == '\n' {
                        self.pos += 2;
                        ln += 1;
                        col = 1;
                        continue;
                    }
                    if in_double_quote {
                        if matches!(next, '"' | '\\' | '$' | '`' | '\n') {
                            if next == '\n' {
                                self.pos += 2;
                                col = 1;
                                ln += 1;
                                continue;
                            }
                            value.push(c);
                            value.push(next);
                            self.pos += 2;
                            col += 2;
                            continue;
                        }
                    } else {
                        if matches!(
                            next,
                            '\\' | '"'
                                | '\''
                                | '`'
                                | '*'
                                | '?'
                                | '['
                                | ']'
                                | '('
                                | ')'
                                | '$'
                                | '-'
                                | '.'
                                | '^'
                                | '+'
                                | '{'
                                | '}'
                        ) {
                            value.push(c);
                            value.push(next);
                        } else {
                            value.push(next);
                        }
                        self.pos += 2;
                        col += 2;
                        continue;
                    }
                }
            }

            // Handle $(...) command substitution
            if c == '$' && self.peek(1) == Some('(') && !in_single_quote {
                self.read_command_substitution(&mut value, &mut col, &mut ln)?;
                continue;
            }

            // Handle ${...} parameter expansion
            if c == '$' && self.peek(1) == Some('{') && !in_single_quote {
                self.read_parameter_expansion(&mut value, &mut col, &mut ln, line, column)?;
                continue;
            }

            // Handle $[...] old-style arithmetic - consume the entire construct
            if c == '$' && self.peek(1) == Some('[') && !in_single_quote {
                value.push(c);
                self.pos += 1;
                col += 1;
                value.push(self.current().unwrap());
                self.pos += 1;
                col += 1;

                let mut depth = 1;
                while depth > 0 && self.pos < self.input.len() {
                    let ch = self.input[self.pos];
                    value.push(ch);
                    if ch == '[' {
                        depth += 1;
                    } else if ch == ']' {
                        depth -= 1;
                    } else if ch == '\n' {
                        ln += 1;
                        col = 0;
                    }
                    self.pos += 1;
                    col += 1;
                }
                continue;
            }

            // Handle special variables $#, $?, $$, etc.
            if c == '$' && !in_single_quote {
                if let Some(next) = self.peek(1) {
                    if matches!(next, '#' | '?' | '$' | '!' | '@' | '*' | '-')
                        || next.is_ascii_digit()
                    {
                        value.push(c);
                        value.push(next);
                        self.pos += 2;
                        col += 2;
                        continue;
                    }
                }
            }

            // Handle backtick command substitution
            if c == '`' && !in_single_quote {
                value.push(c);
                self.pos += 1;
                col += 1;
                while let Some(ch) = self.current() {
                    if ch == '`' {
                        break;
                    }
                    value.push(ch);
                    if ch == '\\' && self.peek(1).is_some() {
                        value.push(self.peek(1).unwrap());
                        self.pos += 1;
                        col += 1;
                    }
                    if ch == '\n' {
                        ln += 1;
                        col = 0;
                    }
                    self.pos += 1;
                    col += 1;
                }
                if self.current() == Some('`') {
                    value.push('`');
                    self.pos += 1;
                    col += 1;
                }
                continue;
            }

            // Regular character
            value.push(c);
            self.pos += 1;
            if c == '\n' {
                ln += 1;
                col = 1;
            } else {
                col += 1;
            }
        }

        self.column = col;
        self.line = ln;

        // Handle content after quote
        if has_content_after_quote && starts_with_quote {
            let open_quote = self.input[start];
            value = format!("{}{}", open_quote, value);
            quoted = false;
            single_quoted = false;
        }

        // Check for unterminated quotes
        if in_single_quote || in_double_quote {
            let quote_type = if in_single_quote { "'" } else { "\"" };
            return Err(LexerError::new(
                format!("unexpected EOF while looking for matching `{}'", quote_type),
                line,
                column,
            ));
        }

        // Check if fully quoted
        if !starts_with_quote && value.len() >= 2 {
            let chars: Vec<char> = value.chars().collect();
            if chars[0] == '\'' && chars[chars.len() - 1] == '\'' {
                let inner: String = chars[1..chars.len() - 1].iter().collect();
                if !inner.contains('\'') && !inner.contains('"') {
                    value = inner;
                    quoted = true;
                    single_quoted = true;
                }
            } else if chars[0] == '"' && chars[chars.len() - 1] == '"' {
                let inner: String = chars[1..chars.len() - 1].iter().collect();
                let mut has_unescaped = false;
                let mut i = 0;
                let inner_chars: Vec<char> = inner.chars().collect();
                while i < inner_chars.len() {
                    if inner_chars[i] == '"' {
                        has_unescaped = true;
                        break;
                    }
                    if inner_chars[i] == '\\' && i + 1 < inner_chars.len() {
                        i += 1;
                    }
                    i += 1;
                }
                if !has_unescaped {
                    value = inner;
                    quoted = true;
                    single_quoted = false;
                }
            }
        }

        if value.is_empty() {
            return Ok(Some(
                Token::new(TokenType::Word, "", start, self.pos, line, column)
                    .with_quotes(quoted, single_quoted),
            ));
        }

        // Check for reserved words
        if !quoted {
            if let Some(&token_type) = RESERVED_WORDS.get(value.as_str()) {
                return Ok(Some(Token::new(
                    token_type, value, start, self.pos, line, column,
                )));
            }
        }

        // Check for assignment
        if !starts_with_quote {
            if let Some(eq_idx) = find_assignment_eq(&value) {
                if eq_idx > 0 && is_valid_assignment_lhs(&value[..eq_idx]) {
                    return Ok(Some(
                        Token::new(
                            TokenType::AssignmentWord,
                            value,
                            start,
                            self.pos,
                            line,
                            column,
                        )
                        .with_quotes(quoted, single_quoted),
                    ));
                }
            }
        }

        // Check for number
        if value.chars().all(|c| c.is_ascii_digit()) {
            return Ok(Some(Token::new(
                TokenType::Number,
                value,
                start,
                self.pos,
                line,
                column,
            )));
        }

        // Check for valid name
        if is_valid_name(&value) {
            return Ok(Some(
                Token::new(TokenType::Name, value, start, self.pos, line, column)
                    .with_quotes(quoted, single_quoted),
            ));
        }

        Ok(Some(
            Token::new(TokenType::Word, value, start, self.pos, line, column)
                .with_quotes(quoted, single_quoted),
        ))
    }

    /// Read $(...) command substitution into value buffer
    fn read_command_substitution(
        &mut self,
        value: &mut String,
        col: &mut usize,
        ln: &mut usize,
    ) -> Result<(), LexerError> {
        let c = self.current().unwrap();
        value.push(c);
        self.pos += 1;
        *col += 1;
        value.push(self.current().unwrap());
        self.pos += 1;
        *col += 1;

        let mut depth = 1;
        let mut in_sq = false;
        let mut in_dq = false;
        let mut case_depth = 0;
        let mut in_case_pattern = false;
        let mut word_buffer = String::new();

        // Check if this is $((...)) arithmetic expansion
        let is_arithmetic =
            self.current() == Some('(') && !self.dollar_dparen_is_subshell(self.pos);

        while depth > 0 && self.pos < self.input.len() {
            let ch = self.input[self.pos];
            value.push(ch);

            if in_sq {
                if ch == '\'' {
                    in_sq = false;
                }
            } else if in_dq {
                if ch == '\\' && self.pos + 1 < self.input.len() {
                    value.push(self.input[self.pos + 1]);
                    self.pos += 1;
                    *col += 1;
                } else if ch == '"' {
                    in_dq = false;
                }
            } else {
                // Not in quotes
                if ch == '\'' {
                    in_sq = true;
                    word_buffer.clear();
                } else if ch == '"' {
                    in_dq = true;
                    word_buffer.clear();
                } else if ch == '\\' && self.pos + 1 < self.input.len() {
                    value.push(self.input[self.pos + 1]);
                    self.pos += 1;
                    *col += 1;
                    word_buffer.clear();
                } else if ch == '$' && self.peek(1) == Some('{') {
                    // Handle ${...} parameter expansion - consume the entire construct
                    self.pos += 1;
                    *col += 1;
                    value.push(self.input[self.pos]); // Add the {
                    self.pos += 1;
                    *col += 1;
                    let mut brace_depth = 1;
                    let mut in_brace_sq = false;
                    let mut in_brace_dq = false;
                    while brace_depth > 0 && self.pos < self.input.len() {
                        let bc = self.input[self.pos];
                        if bc == '\\' && self.pos + 1 < self.input.len() && !in_brace_sq {
                            value.push(bc);
                            self.pos += 1;
                            *col += 1;
                            value.push(self.input[self.pos]);
                            self.pos += 1;
                            *col += 1;
                            continue;
                        }
                        value.push(bc);
                        if in_brace_sq {
                            if bc == '\'' {
                                in_brace_sq = false;
                            }
                        } else if in_brace_dq {
                            if bc == '"' {
                                in_brace_dq = false;
                            }
                        } else {
                            match bc {
                                '\'' => in_brace_sq = true,
                                '"' => in_brace_dq = true,
                                '{' => brace_depth += 1,
                                '}' => brace_depth -= 1,
                                _ => {}
                            }
                        }
                        if bc == '\n' {
                            *ln += 1;
                            *col = 0;
                        } else {
                            *col += 1;
                        }
                        self.pos += 1;
                    }
                    word_buffer.clear();
                    continue;
                } else if ch == '#'
                    && !is_arithmetic
                    && (word_buffer.is_empty()
                        || self
                            .input
                            .get(self.pos.wrapping_sub(1))
                            .map_or(false, |c| c.is_whitespace()))
                {
                    // Comment - skip to end of line (only in command substitution, not arithmetic)
                    while self.pos + 1 < self.input.len()
                        && self.input[self.pos + 1] != '\n'
                    {
                        self.pos += 1;
                        *col += 1;
                        value.push(self.input[self.pos]);
                    }
                    word_buffer.clear();
                } else if ch.is_ascii_alphabetic() || ch == '_' {
                    word_buffer.push(ch);
                } else {
                    // Check for keywords
                    if word_buffer == "case" {
                        case_depth += 1;
                        in_case_pattern = false;
                    } else if word_buffer == "in" && case_depth > 0 {
                        in_case_pattern = true;
                    } else if word_buffer == "esac" && case_depth > 0 {
                        case_depth -= 1;
                        in_case_pattern = false;
                    }
                    word_buffer.clear();

                    if ch == '(' {
                        // Check for $( which starts nested command substitution
                        if self.pos > 0
                            && self.input.get(self.pos.wrapping_sub(1)) == Some(&'$')
                        {
                            depth += 1;
                        } else if !in_case_pattern {
                            depth += 1;
                        }
                    } else if ch == ')' {
                        if in_case_pattern {
                            in_case_pattern = false;
                        } else {
                            depth -= 1;
                        }
                    } else if ch == ';' {
                        // ;; in case body means next pattern
                        if case_depth > 0 && self.peek(1) == Some(';') {
                            in_case_pattern = true;
                        }
                    }
                }
            }

            if ch == '\n' {
                *ln += 1;
                *col = 0;
                word_buffer.clear();
            }
            self.pos += 1;
            *col += 1;
        }
        Ok(())
    }

    /// Read ${...} parameter expansion into value buffer
    fn read_parameter_expansion(
        &mut self,
        value: &mut String,
        col: &mut usize,
        ln: &mut usize,
        start_line: usize,
        start_column: usize,
    ) -> Result<(), LexerError> {
        let c = self.current().unwrap();
        value.push(c);
        self.pos += 1;
        *col += 1;
        value.push(self.current().unwrap());
        self.pos += 1;
        *col += 1;

        let mut depth = 1;
        let mut in_param_sq = false;
        let mut in_param_dq = false;
        let mut single_quote_start_line = *ln;
        let mut single_quote_start_col = *col;
        let mut double_quote_start_line = *ln;
        let mut double_quote_start_col = *col;

        while depth > 0 && self.pos < self.input.len() {
            let ch = self.input[self.pos];

            // Handle backslash-newline line continuation
            if ch == '\\' && self.peek(1) == Some('\n') {
                self.pos += 2;
                *ln += 1;
                *col = 1;
                continue;
            }

            if ch == '\\' && self.pos + 1 < self.input.len() && !in_param_sq {
                value.push(ch);
                self.pos += 1;
                *col += 1;
                value.push(self.input[self.pos]);
                self.pos += 1;
                *col += 1;
                continue;
            }

            value.push(ch);

            if in_param_sq {
                if ch == '\'' {
                    in_param_sq = false;
                }
            } else if in_param_dq {
                if ch == '"' {
                    in_param_dq = false;
                }
            } else {
                match ch {
                    '\'' => {
                        in_param_sq = true;
                        single_quote_start_line = *ln;
                        single_quote_start_col = *col;
                    }
                    '"' => {
                        in_param_dq = true;
                        double_quote_start_line = *ln;
                        double_quote_start_col = *col;
                    }
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
            }

            if ch == '\n' {
                *ln += 1;
                *col = 0;
            }
            self.pos += 1;
            *col += 1;
        }

        // Check for unterminated quotes inside ${...}
        if in_param_sq {
            return Err(LexerError::new(
                "unexpected EOF while looking for matching `''",
                single_quote_start_line,
                single_quote_start_col,
            ));
        }
        if in_param_dq {
            return Err(LexerError::new(
                "unexpected EOF while looking for matching `\"'",
                double_quote_start_line,
                double_quote_start_col,
            ));
        }

        // Suppress unused variable warnings
        let _ = start_line;
        let _ = start_column;

        Ok(())
    }

    pub(crate) fn read_word_with_brace_expansion(
        &mut self,
        start: usize,
        line: usize,
        column: usize,
    ) -> Result<Option<Token>, LexerError> {
        let mut col = column;

        while self.pos < self.input.len() {
            let c = self.input[self.pos];

            if is_word_boundary(c) {
                break;
            }

            if c == '{' {
                if self.scan_brace_expansion(self.pos).is_some() {
                    let mut depth = 1;
                    self.pos += 1;
                    col += 1;
                    while self.pos < self.input.len() && depth > 0 {
                        match self.input[self.pos] {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        self.pos += 1;
                        col += 1;
                    }
                    continue;
                }
                self.pos += 1;
                col += 1;
                continue;
            }

            if c == '}' {
                self.pos += 1;
                col += 1;
                continue;
            }

            if c == '$' && self.peek(1) == Some('(') {
                self.pos += 1;
                col += 1;
                self.pos += 1;
                col += 1;
                let mut depth = 1;
                while depth > 0 && self.pos < self.input.len() {
                    match self.input[self.pos] {
                        '(' => depth += 1,
                        ')' => depth -= 1,
                        _ => {}
                    }
                    self.pos += 1;
                    col += 1;
                }
                continue;
            }

            if c == '$' && self.peek(1) == Some('{') {
                self.pos += 1;
                col += 1;
                self.pos += 1;
                col += 1;
                let mut depth = 1;
                while depth > 0 && self.pos < self.input.len() {
                    match self.input[self.pos] {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    self.pos += 1;
                    col += 1;
                }
                continue;
            }

            if c == '`' {
                self.pos += 1;
                col += 1;
                while self.pos < self.input.len() && self.input[self.pos] != '`' {
                    if self.input[self.pos] == '\\' && self.pos + 1 < self.input.len() {
                        self.pos += 2;
                        col += 2;
                    } else {
                        self.pos += 1;
                        col += 1;
                    }
                }
                if self.pos < self.input.len() {
                    self.pos += 1;
                    col += 1;
                }
                continue;
            }

            self.pos += 1;
            col += 1;
        }

        let value: String = self.input[start..self.pos].iter().collect();
        self.column = col;

        Ok(Some(
            Token::new(TokenType::Word, value, start, self.pos, line, column)
                .with_quotes(false, false),
        ))
    }
}
