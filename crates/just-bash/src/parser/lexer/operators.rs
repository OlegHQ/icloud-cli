//! Operator tables and helper functions for the bash lexer

use std::collections::HashMap;

use super::token::TokenType;

lazy_static::lazy_static! {
    /// Reserved words in bash
    pub(crate) static ref RESERVED_WORDS: HashMap<&'static str, TokenType> = {
        let mut m = HashMap::new();
        m.insert("if", TokenType::If);
        m.insert("then", TokenType::Then);
        m.insert("else", TokenType::Else);
        m.insert("elif", TokenType::Elif);
        m.insert("fi", TokenType::Fi);
        m.insert("for", TokenType::For);
        m.insert("while", TokenType::While);
        m.insert("until", TokenType::Until);
        m.insert("do", TokenType::Do);
        m.insert("done", TokenType::Done);
        m.insert("case", TokenType::Case);
        m.insert("esac", TokenType::Esac);
        m.insert("in", TokenType::In);
        m.insert("function", TokenType::Function);
        m.insert("select", TokenType::Select);
        m.insert("time", TokenType::Time);
        m.insert("coproc", TokenType::Coproc);
        m
    };

    /// Single-character operators
    pub(crate) static ref SINGLE_CHAR_OPS: HashMap<char, TokenType> = {
        let mut m = HashMap::new();
        m.insert('|', TokenType::Pipe);
        m.insert('&', TokenType::Amp);
        m.insert(';', TokenType::Semicolon);
        m.insert('(', TokenType::LParen);
        m.insert(')', TokenType::RParen);
        m.insert('<', TokenType::Less);
        m.insert('>', TokenType::Great);
        m
    };
}

/// Three-character operators
pub(crate) const THREE_CHAR_OPS: &[(&str, TokenType)] = &[
    (";;&", TokenType::SemiSemiAnd),
    ("<<<", TokenType::TLess),
    ("&>>", TokenType::AndDGreat),
];

/// Two-character operators
pub(crate) const TWO_CHAR_OPS: &[(&str, TokenType)] = &[
    ("[[", TokenType::DBrackStart),
    ("]]", TokenType::DBrackEnd),
    ("((", TokenType::DParenStart),
    ("))", TokenType::DParenEnd),
    ("&&", TokenType::AndAnd),
    ("||", TokenType::OrOr),
    (";;", TokenType::DSemi),
    (";&", TokenType::SemiAnd),
    ("|&", TokenType::PipeAmp),
    (">>", TokenType::DGreat),
    ("<&", TokenType::LessAnd),
    (">&", TokenType::GreatAnd),
    ("<>", TokenType::LessGreat),
    (">|", TokenType::Clobber),
    ("&>", TokenType::AndGreat),
];

/// Check if a string is a valid variable name
pub(crate) fn is_valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

/// Check if a character is a word boundary (ends a word token)
pub(crate) fn is_word_boundary(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '<' | '>'
    )
}

/// Check if a string is a valid assignment LHS with optional nested array subscript
pub(crate) fn is_valid_assignment_lhs(s: &str) -> bool {
    // Must start with valid variable name
    let name_end = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .count();

    if name_end == 0 {
        return false;
    }

    // Check first char is letter or underscore
    let first = s.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }

    let after_name = &s[name_end..];

    // If nothing after name, it's valid (simple variable)
    if after_name.is_empty() || after_name == "+" {
        return true;
    }

    // If it's an array subscript, need to check for balanced brackets
    if after_name.starts_with('[') {
        let mut depth = 0;
        let mut i = 0;
        for c in after_name.chars() {
            if c == '[' {
                depth += 1;
            } else if c == ']' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            i += c.len_utf8();
        }
        // Must have found closing bracket
        if depth != 0 {
            return false;
        }
        // After closing bracket, only + is allowed (for +=)
        let after_bracket = &after_name[i + 1..];
        return after_bracket.is_empty() || after_bracket == "+";
    }

    false
}

/// Find the index of assignment '=' or '+=' outside of brackets.
pub(crate) fn find_assignment_eq(s: &str) -> Option<usize> {
    let mut depth = 0;
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            '=' if depth == 0 => return Some(i),
            '+' if depth == 0 && chars.get(i + 1) == Some(&'=') => return Some(i + 1),
            _ => {}
        }
    }
    None
}
