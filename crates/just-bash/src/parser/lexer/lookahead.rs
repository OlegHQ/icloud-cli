//! Lookahead and scanning helpers for the lexer
//!
//! Contains methods for disambiguating constructs like `((` vs nested subshells,
//! `$((` vs `$( (`, FD variables, extglob patterns, and brace expansions.

use super::operators::is_word_boundary;
use super::Lexer;

pub(crate) struct ExtglobResult {
    pub(crate) content: String,
    pub(crate) end: usize,
}

pub(crate) struct FdVariableResult {
    pub(crate) varname: String,
    pub(crate) end: usize,
}

impl Lexer {
    pub(crate) fn looks_like_nested_subshells(&self, start_pos: usize) -> bool {
        let mut pos = start_pos;

        // Skip optional whitespace
        while pos < self.input.len() && matches!(self.input.get(pos), Some(' ' | '\t')) {
            pos += 1;
        }

        if pos >= self.input.len() {
            return false;
        }

        let c = self.input[pos];

        // If we see another ( immediately, recursively check
        if c == '(' {
            return self.looks_like_nested_subshells(pos + 1);
        }

        // Check if this looks like the start of a command name
        let is_letter = c.is_ascii_alphabetic() || c == '_';
        let is_special_command = c == '!' || c == '[';

        if !is_letter && !is_special_command {
            return false;
        }

        // Read the word-like content
        let mut word_end = pos;
        while word_end < self.input.len() {
            let ch = self.input[word_end];
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
                word_end += 1;
            } else {
                break;
            }
        }

        if word_end == pos {
            return is_special_command;
        }

        // Skip whitespace after the word
        let mut after_word = word_end;
        while after_word < self.input.len()
            && matches!(self.input.get(after_word), Some(' ' | '\t'))
        {
            after_word += 1;
        }

        if after_word >= self.input.len() {
            return false;
        }

        let next_char = self.input[after_word];

        // If followed by =, it's likely arithmetic
        if next_char == '=' && self.input.get(after_word + 1) != Some(&'=') {
            return false;
        }

        // If followed by newline, not a proper subshell pattern
        if next_char == '\n' {
            return false;
        }

        // If followed by arithmetic operators without space, likely arithmetic
        if word_end == after_word
            && matches!(
                next_char,
                '+' | '*' | '/' | '%' | '<' | '>' | '&' | '|' | '^' | '!' | '~' | '?' | ':'
            )
            && next_char != '-'
        {
            return false;
        }

        // If followed by )), it's arithmetic
        if next_char == ')' && self.input.get(after_word + 1) == Some(&')') {
            return false;
        }

        // If followed by command-like arguments after whitespace, it's likely a command
        if after_word > word_end
            && (next_char == '-'
                || next_char == '"'
                || next_char == '\''
                || next_char == '$'
                || next_char.is_ascii_alphabetic()
                || next_char == '_'
                || next_char == '/'
                || next_char == '.')
        {
            // Scan ahead to find ) on the same line
            let mut scan_pos = after_word;
            while scan_pos < self.input.len() && self.input[scan_pos] != '\n' {
                if self.input[scan_pos] == ')' {
                    return true;
                }
                scan_pos += 1;
            }
            // No ) found on this line - not a proper subshell
            return false;
        }

        // If followed by ) then || or &&, it's nested subshells
        if next_char == ')' {
            let mut after_paren = after_word + 1;
            while after_paren < self.input.len()
                && matches!(self.input.get(after_paren), Some(' ' | '\t'))
            {
                after_paren += 1;
            }
            if (self.input.get(after_paren) == Some(&'|')
                && self.input.get(after_paren + 1) == Some(&'|'))
                || (self.input.get(after_paren) == Some(&'&')
                    && self.input.get(after_paren + 1) == Some(&'&'))
                || self.input.get(after_paren) == Some(&';')
                || (self.input.get(after_paren) == Some(&'|')
                    && self.input.get(after_paren + 1) != Some(&'|'))
            {
                return true;
            }
        }

        false
    }

    pub(crate) fn dparen_closes_with_spaced_parens(&self, start_pos: usize) -> bool {
        let mut pos = start_pos;
        let mut depth = 2;
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        while pos < self.input.len() && depth > 0 {
            let c = self.input[pos];

            if in_single_quote {
                if c == '\'' {
                    in_single_quote = false;
                }
                pos += 1;
                continue;
            }

            if in_double_quote {
                if c == '\\' && pos + 1 < self.input.len() {
                    pos += 2;
                    continue;
                }
                if c == '"' {
                    in_double_quote = false;
                }
                pos += 1;
                continue;
            }

            match c {
                '\'' => in_single_quote = true,
                '"' => in_double_quote = true,
                '\\' if pos + 1 < self.input.len() => {
                    pos += 2;
                    continue;
                }
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 1 {
                        // Check if next char is ) with whitespace
                        let next_pos = pos + 1;
                        if self.input.get(next_pos) == Some(&')') {
                            return false;
                        }
                        let mut scan_pos = next_pos;
                        let mut has_whitespace = false;
                        while scan_pos < self.input.len()
                            && matches!(self.input.get(scan_pos), Some(' ' | '\t' | '\n'))
                        {
                            has_whitespace = true;
                            scan_pos += 1;
                        }
                        if has_whitespace && self.input.get(scan_pos) == Some(&')') {
                            return true;
                        }
                    }
                    if depth == 0 {
                        return false;
                    }
                }
                '|' if depth == 1 => {
                    if self.input.get(pos + 1) == Some(&'|') {
                        return true;
                    }
                    if self.input.get(pos + 1) != Some(&'|') {
                        return true;
                    }
                }
                '&' if depth == 1 && self.input.get(pos + 1) == Some(&'&') => {
                    return true;
                }
                _ => {}
            }
            pos += 1;
        }

        false
    }

    /// Scan ahead from a $(( position to determine if it should be treated as
    /// $( ( subshell ) ) instead of $(( arithmetic )).
    pub(crate) fn dollar_dparen_is_subshell(&self, start_pos: usize) -> bool {
        let mut pos = start_pos + 1; // Skip the second (
        let mut depth = 2; // We've seen $((, so we start at depth 2
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut has_newline = false;

        while pos < self.input.len() && depth > 0 {
            let c = self.input[pos];

            if in_single_quote {
                if c == '\'' {
                    in_single_quote = false;
                }
                if c == '\n' {
                    has_newline = true;
                }
                pos += 1;
                continue;
            }

            if in_double_quote {
                if c == '\\' && pos + 1 < self.input.len() {
                    // Skip escaped char
                    pos += 2;
                    continue;
                }
                if c == '"' {
                    in_double_quote = false;
                }
                if c == '\n' {
                    has_newline = true;
                }
                pos += 1;
                continue;
            }

            // Not in quotes
            match c {
                '\'' => {
                    in_single_quote = true;
                    pos += 1;
                }
                '"' => {
                    in_double_quote = true;
                    pos += 1;
                }
                '\\' if pos + 1 < self.input.len() => {
                    // Skip escaped char
                    pos += 2;
                }
                '\n' => {
                    has_newline = true;
                    pos += 1;
                }
                '(' => {
                    depth += 1;
                    pos += 1;
                }
                ')' => {
                    depth -= 1;
                    if depth == 1 {
                        // We've closed the inner subshell. Check what follows.
                        let next_pos = pos + 1;
                        if next_pos < self.input.len() && self.input[next_pos] == ')' {
                            // )) - adjacent parens = arithmetic
                            return false;
                        }
                        // Check if there's whitespace followed by )
                        let mut scan_pos = next_pos;
                        let mut has_whitespace = false;
                        while scan_pos < self.input.len()
                            && matches!(self.input.get(scan_pos), Some(' ' | '\t' | '\n'))
                        {
                            has_whitespace = true;
                            scan_pos += 1;
                        }
                        if has_whitespace
                            && scan_pos < self.input.len()
                            && self.input[scan_pos] == ')'
                        {
                            // This is ) ) with whitespace - subshell
                            return true;
                        }
                        // If it has newlines, treat as subshell
                        if has_newline {
                            return true;
                        }
                    }
                    if depth == 0 {
                        return false;
                    }
                    pos += 1;
                }
                _ => {
                    pos += 1;
                }
            }
        }

        // Didn't find a definitive answer - default to arithmetic behavior
        false
    }

    pub(crate) fn scan_fd_variable(&self, start_pos: usize) -> Option<FdVariableResult> {
        let mut pos = start_pos + 1;

        // Scan variable name
        let name_start = pos;
        while pos < self.input.len() {
            let c = self.input[pos];
            if pos == name_start {
                if !c.is_ascii_alphabetic() && c != '_' {
                    return None;
                }
            } else if !c.is_ascii_alphanumeric() && c != '_' {
                break;
            }
            pos += 1;
        }

        if pos == name_start {
            return None;
        }

        let varname: String = self.input[name_start..pos].iter().collect();

        // Must be followed by closing brace
        if pos >= self.input.len() || self.input[pos] != '}' {
            return None;
        }
        pos += 1;

        // Must be immediately followed by a redirect operator
        if pos >= self.input.len() {
            return None;
        }

        let c = self.input[pos];
        let c2 = self.input.get(pos + 1).copied();

        let is_redirect_op = c == '>' || c == '<' || (c == '&' && matches!(c2, Some('>' | '<')));

        if !is_redirect_op {
            return None;
        }

        Some(FdVariableResult { varname, end: pos })
    }

    pub(crate) fn scan_extglob_pattern(&self, start_pos: usize) -> Option<ExtglobResult> {
        let mut pos = start_pos + 1;
        let mut depth = 1;

        while pos < self.input.len() && depth > 0 {
            let c = self.input[pos];

            if c == '\\' && pos + 1 < self.input.len() {
                pos += 2;
                continue;
            }

            if "@*+?!".contains(c) && pos + 1 < self.input.len() && self.input[pos + 1] == '(' {
                pos += 1;
                depth += 1;
                pos += 1;
                continue;
            }

            match c {
                '(' => {
                    depth += 1;
                    pos += 1;
                }
                ')' => {
                    depth -= 1;
                    pos += 1;
                }
                '\n' => return None,
                _ => pos += 1,
            }
        }

        if depth == 0 {
            Some(ExtglobResult {
                content: self.input[start_pos..pos].iter().collect(),
                end: pos,
            })
        } else {
            None
        }
    }

    pub(crate) fn scan_brace_expansion(&self, start_pos: usize) -> Option<String> {
        let mut pos = start_pos + 1;
        let mut depth = 1;
        let mut has_comma = false;
        let mut has_range = false;

        while pos < self.input.len() && depth > 0 {
            let c = self.input[pos];

            match c {
                '{' => {
                    depth += 1;
                    pos += 1;
                }
                '}' => {
                    depth -= 1;
                    pos += 1;
                }
                ',' if depth == 1 => {
                    has_comma = true;
                    pos += 1;
                }
                '.' if pos + 1 < self.input.len() && self.input[pos + 1] == '.' => {
                    has_range = true;
                    pos += 2;
                }
                ' ' | '\t' | '\n' | ';' | '&' | '|' => return None,
                _ => pos += 1,
            }
        }

        if depth == 0 && (has_comma || has_range) {
            Some(self.input[start_pos..pos].iter().collect())
        } else {
            None
        }
    }

    pub(crate) fn scan_literal_brace_word(&self, start_pos: usize) -> Option<String> {
        let mut pos = start_pos + 1;
        let mut depth = 1;

        while pos < self.input.len() && depth > 0 {
            let c = self.input[pos];

            match c {
                '{' => {
                    depth += 1;
                    pos += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(self.input[start_pos..=pos].iter().collect());
                    }
                    pos += 1;
                }
                ' ' | '\t' | '\n' | ';' | '&' | '|' => return None,
                _ => pos += 1,
            }
        }

        None
    }

    pub(crate) fn is_word_char_following(&self, pos: usize) -> bool {
        if pos >= self.input.len() {
            return false;
        }
        let c = self.input[pos];
        !is_word_boundary(c)
    }
}
