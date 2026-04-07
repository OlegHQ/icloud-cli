//! Word Part Helper Functions
//!
//! Provides common operations on WordPiece types to eliminate duplication
//! across expansion and word parsing.

use brush_parser::word::WordPiece;

/// Get the text string value from a word piece.
/// Returns the value for Text, SingleQuotedText, and EscapeSequence pieces.
/// Returns None for complex pieces that require expansion.
pub fn get_literal_value(piece: &WordPiece) -> Option<&str> {
    match piece {
        WordPiece::Text(s) => Some(s),
        WordPiece::SingleQuotedText(s) => Some(s),
        WordPiece::EscapeSequence(s) => Some(s),
        _ => None,
    }
}

/// Check if a word piece is "quoted" - meaning glob characters should be treated literally.
/// A piece is quoted if it is:
/// - SingleQuotedText
/// - EscapeSequence
/// - DoubleQuotedSequence (entirely quoted)
/// - AnsiCQuotedText
/// - Text with empty value (doesn't affect quoting)
pub fn is_quoted_part(piece: &WordPiece) -> bool {
    match piece {
        WordPiece::SingleQuotedText(_) => true,
        WordPiece::EscapeSequence(_) => true,
        WordPiece::DoubleQuotedSequence(_) => true,
        WordPiece::AnsiCQuotedText(_) => true,
        WordPiece::Text(s) => s.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_literal_value() {
        assert_eq!(
            get_literal_value(&WordPiece::Text("hello".to_string())),
            Some("hello")
        );
        assert_eq!(
            get_literal_value(&WordPiece::SingleQuotedText("world".to_string())),
            Some("world")
        );
        assert_eq!(
            get_literal_value(&WordPiece::EscapeSequence("n".to_string())),
            Some("n")
        );
        assert_eq!(
            get_literal_value(&WordPiece::CommandSubstitution("cmd".to_string())),
            None
        );
    }

    #[test]
    fn test_is_quoted_part() {
        assert!(is_quoted_part(&WordPiece::SingleQuotedText(
            "test".to_string()
        )));
        assert!(is_quoted_part(&WordPiece::EscapeSequence("n".to_string())));
        assert!(is_quoted_part(&WordPiece::DoubleQuotedSequence(vec![])));
        assert!(is_quoted_part(&WordPiece::AnsiCQuotedText(
            "test".to_string()
        )));
        assert!(is_quoted_part(&WordPiece::Text("".to_string())));
        assert!(!is_quoted_part(&WordPiece::Text("test".to_string())));
        assert!(!is_quoted_part(&WordPiece::CommandSubstitution(
            "cmd".to_string()
        )));
    }
}
