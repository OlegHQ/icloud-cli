//! Token types and structures for the bash lexer

/// Token types for bash lexer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenType {
    // End of input
    Eof,

    // Newlines and separators
    Newline,
    Semicolon,
    Amp, // &

    // Operators
    Pipe,    // |
    PipeAmp, // |&
    AndAnd,  // &&
    OrOr,    // ||
    Bang,    // !

    // Redirections
    Less,      // <
    Great,     // >
    DLess,     // <<
    DGreat,    // >>
    LessAnd,   // <&
    GreatAnd,  // >&
    LessGreat, // <>
    DLessDash, // <<-
    Clobber,   // >|
    TLess,     // <<<
    AndGreat,  // &>
    AndDGreat, // &>>

    // Grouping
    LParen, // (
    RParen, // )
    LBrace, // {
    RBrace, // }

    // Special
    DSemi,       // ;;
    SemiAnd,     // ;&
    SemiSemiAnd, // ;;&

    // Compound commands
    DBrackStart, // [[
    DBrackEnd,   // ]]
    DParenStart, // ((
    DParenEnd,   // ))

    // Reserved words
    If,
    Then,
    Else,
    Elif,
    Fi,
    For,
    While,
    Until,
    Do,
    Done,
    Case,
    Esac,
    In,
    Function,
    Select,
    Time,
    Coproc,

    // Words and identifiers
    Word,
    Name,           // Valid variable name
    Number,         // For redirections like 2>&1
    AssignmentWord, // VAR=value
    FdVariable,     // {varname} before redirect operator

    // Comments
    Comment,

    // Here-document content
    HeredocContent,
}

impl TokenType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Eof => "EOF",
            Self::Newline => "NEWLINE",
            Self::Semicolon => ";",
            Self::Amp => "&",
            Self::Pipe => "|",
            Self::PipeAmp => "|&",
            Self::AndAnd => "&&",
            Self::OrOr => "||",
            Self::Bang => "!",
            Self::Less => "<",
            Self::Great => ">",
            Self::DLess => "<<",
            Self::DGreat => ">>",
            Self::LessAnd => "<&",
            Self::GreatAnd => ">&",
            Self::LessGreat => "<>",
            Self::DLessDash => "<<-",
            Self::Clobber => ">|",
            Self::TLess => "<<<",
            Self::AndGreat => "&>",
            Self::AndDGreat => "&>>",
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LBrace => "{",
            Self::RBrace => "}",
            Self::DSemi => ";;",
            Self::SemiAnd => ";&",
            Self::SemiSemiAnd => ";;&",
            Self::DBrackStart => "[[",
            Self::DBrackEnd => "]]",
            Self::DParenStart => "((",
            Self::DParenEnd => "))",
            Self::If => "if",
            Self::Then => "then",
            Self::Else => "else",
            Self::Elif => "elif",
            Self::Fi => "fi",
            Self::For => "for",
            Self::While => "while",
            Self::Until => "until",
            Self::Do => "do",
            Self::Done => "done",
            Self::Case => "case",
            Self::Esac => "esac",
            Self::In => "in",
            Self::Function => "function",
            Self::Select => "select",
            Self::Time => "time",
            Self::Coproc => "coproc",
            Self::Word => "WORD",
            Self::Name => "NAME",
            Self::Number => "NUMBER",
            Self::AssignmentWord => "ASSIGNMENT_WORD",
            Self::FdVariable => "FD_VARIABLE",
            Self::Comment => "COMMENT",
            Self::HeredocContent => "HEREDOC_CONTENT",
        }
    }
}

/// A token produced by the lexer
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub token_type: TokenType,
    pub value: String,
    /// Original position in input
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
    /// For WORD tokens: quote information
    pub quoted: bool,
    pub single_quoted: bool,
}

impl Token {
    pub fn new(
        token_type: TokenType,
        value: impl Into<String>,
        start: usize,
        end: usize,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            token_type,
            value: value.into(),
            start,
            end,
            line,
            column,
            quoted: false,
            single_quoted: false,
        }
    }

    pub fn with_quotes(mut self, quoted: bool, single_quoted: bool) -> Self {
        self.quoted = quoted;
        self.single_quoted = single_quoted;
        self
    }
}

/// Error thrown when the lexer encounters invalid input
#[derive(Debug, Clone)]
pub struct LexerError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for LexerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for LexerError {}

impl LexerError {
    pub fn new(message: impl Into<String>, line: usize, column: usize) -> Self {
        Self {
            message: message.into(),
            line,
            column,
        }
    }
}
