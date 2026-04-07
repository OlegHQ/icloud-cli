//! Command Substitution Helpers
//!
//! Helper functions for handling command substitution patterns.

use brush_parser::ast as bast;

/// Check if a command substitution body matches the $(<file) shorthand pattern.
/// This is a special case where $(< file) is equivalent to $(cat file) but reads
/// the file directly without spawning a subprocess.
///
/// For this to match, the body must consist of:
/// - One complete command with no operators (no && or ||)
/// - One pipeline with one command
/// - A SimpleCommand with no name, no args, no assignments
/// - Exactly one input redirection (<)
///
/// Note: The special $(<file) behavior only works when it's the ONLY element
/// in the command substitution. $(< file; cmd) or $(cmd; < file) are NOT special.
///
/// In brush-parser, the command substitution body is a String. We parse it
/// into a `Program` and inspect the resulting AST.
pub fn get_file_read_shorthand(body: &bast::Program) -> Option<&bast::Word> {
    // Must have exactly one complete command
    if body.complete_commands.len() != 1 {
        return None;
    }

    let complete_command = &body.complete_commands[0];
    // CompoundList(Vec<CompoundListItem>) - must have exactly one item
    if complete_command.0.len() != 1 {
        return None;
    }

    let item = &complete_command.0[0];
    // CompoundListItem(AndOrList, SeparatorOperator)
    let and_or_list = &item.0;
    // Must not have any additional operators (no && or ||)
    if !and_or_list.additional.is_empty() {
        return None;
    }

    let pipeline = &and_or_list.first;
    // Pipeline must not be negated
    if pipeline.bang {
        return None;
    }
    // Must have exactly one command
    if pipeline.seq.len() != 1 {
        return None;
    }

    let cmd = &pipeline.seq[0];
    // Must be a Simple command
    let simple_cmd = match cmd {
        bast::Command::Simple(sc) => sc,
        _ => return None,
    };

    // Must have no command name
    if simple_cmd.word_or_name.is_some() {
        return None;
    }

    // Collect all prefix/suffix items
    let mut redirects = Vec::new();
    let mut has_words = false;
    let mut has_assignments = false;

    if let Some(prefix) = &simple_cmd.prefix {
        for item in &prefix.0 {
            match item {
                bast::CommandPrefixOrSuffixItem::IoRedirect(r) => redirects.push(r),
                bast::CommandPrefixOrSuffixItem::Word(_) => has_words = true,
                bast::CommandPrefixOrSuffixItem::AssignmentWord(_, _) => has_assignments = true,
                bast::CommandPrefixOrSuffixItem::ProcessSubstitution(_, _) => return None,
            }
        }
    }

    if let Some(suffix) = &simple_cmd.suffix {
        for item in &suffix.0 {
            match item {
                bast::CommandPrefixOrSuffixItem::IoRedirect(r) => redirects.push(r),
                bast::CommandPrefixOrSuffixItem::Word(_) => has_words = true,
                bast::CommandPrefixOrSuffixItem::AssignmentWord(_, _) => has_assignments = true,
                bast::CommandPrefixOrSuffixItem::ProcessSubstitution(_, _) => return None,
            }
        }
    }

    // Must have no arguments and no assignments
    if has_words || has_assignments {
        return None;
    }

    // Must have exactly one redirection
    if redirects.len() != 1 {
        return None;
    }

    // Must be an input redirection (<) targeting a filename
    match redirects[0] {
        bast::IoRedirect::File(_, bast::IoFileRedirectKind::Read, bast::IoFileRedirectTarget::Filename(ref word)) => {
            Some(word)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brush_parser::{Parser, ParserOptions, SourceInfo};

    fn parse_program(input: &str) -> bast::Program {
        let reader = std::io::Cursor::new(input.to_string());
        let mut parser = Parser::new(
            std::io::BufReader::new(reader),
            &ParserOptions::default(),
            &SourceInfo::default(),
        );
        parser.parse_program().unwrap()
    }

    #[test]
    fn test_file_read_shorthand() {
        // This would be the body of $(< file)
        let program = parse_program("< file");
        let result = get_file_read_shorthand(&program);
        assert!(result.is_some());
        assert_eq!(result.unwrap().value, "file");
    }

    #[test]
    fn test_not_shorthand_with_command() {
        let program = parse_program("cat file");
        let result = get_file_read_shorthand(&program);
        assert!(result.is_none());
    }

    #[test]
    fn test_not_shorthand_with_multiple_statements() {
        let program = parse_program("< file; echo done");
        let result = get_file_read_shorthand(&program);
        assert!(result.is_none());
    }

    #[test]
    fn test_not_shorthand_with_output_redirect() {
        let program = parse_program("> file");
        let result = get_file_read_shorthand(&program);
        assert!(result.is_none());
    }
}
