//! Alias Expansion
//!
//! Handles bash alias expansion for SimpleCommands (brush_parser::ast types).
//!
//! Alias expansion rules:
//! 1. Only expands if command name is a literal unquoted word
//! 2. Alias value is substituted for the command name
//! 3. If alias value ends with a space, the next word is also checked for alias expansion
//! 4. Recursive expansion is allowed but limited to prevent infinite loops

use brush_parser::ast as bast;
use std::collections::{HashMap, HashSet};

/// Alias prefix used in environment variables
pub const ALIAS_PREFIX: &str = "BASH_ALIAS_";

/// Context needed for alias expansion operations
pub struct AliasExpansionContext<'a> {
    pub env: &'a HashMap<String, String>,
}

/// Check if a word is a literal unquoted word (eligible for alias expansion).
/// In brush_parser, a Word's `value` is the raw source text. A literal unquoted
/// word contains no quoting or expansion characters.
pub fn is_literal_unquoted_word(word: &bast::Word) -> bool {
    let v = &word.value;
    !v.is_empty()
        && !v.contains(|c: char| matches!(c,
            '"' | '\'' | '$' | '`' | '\\' | '*' | '?' | '[' | ']'
            | '{' | '}' | '(' | ')' | '<' | '>' | '|' | '&' | ';'
            | '#' | '!' | '~'
        ))
}

/// Get the literal value of a word if it's a simple literal (no quoting/expansion).
pub fn get_literal_value(word: &bast::Word) -> Option<&str> {
    if is_literal_unquoted_word(word) {
        Some(&word.value)
    } else {
        None
    }
}

/// Get the alias value for a name, if defined
pub fn get_alias<'a>(ctx: &'a AliasExpansionContext<'a>, name: &str) -> Option<&'a str> {
    let key = format!("{}{}", ALIAS_PREFIX, name);
    ctx.env.get(&key).map(|s| s.as_str())
}

/// Check if an alias is defined for a name
pub fn has_alias(ctx: &AliasExpansionContext, name: &str) -> bool {
    let key = format!("{}{}", ALIAS_PREFIX, name);
    ctx.env.contains_key(&key)
}

/// Set an alias in the environment
pub fn set_alias(env: &mut HashMap<String, String>, name: &str, value: &str) {
    let key = format!("{}{}", ALIAS_PREFIX, name);
    env.insert(key, value.to_string());
}

/// Remove an alias from the environment
pub fn unset_alias(env: &mut HashMap<String, String>, name: &str) -> bool {
    let key = format!("{}{}", ALIAS_PREFIX, name);
    env.remove(&key).is_some()
}

/// Get all defined aliases as (name, value) pairs
pub fn get_all_aliases(env: &HashMap<String, String>) -> Vec<(String, String)> {
    env.iter()
        .filter_map(|(k, v)| {
            k.strip_prefix(ALIAS_PREFIX)
                .map(|name| (name.to_string(), v.clone()))
        })
        .collect()
}

/// Helper: parse a command string into a brush_parser Program AST.
fn parse_command(input: &str) -> Result<bast::Program, String> {
    let tokens = brush_parser::tokenize_str(input).map_err(|e| e.to_string())?;
    brush_parser::parse_tokens(
        &tokens,
        &brush_parser::ParserOptions::default(),
        &brush_parser::SourceInfo::default(),
    )
    .map_err(|e| e.to_string())
}

/// Helper: extract command name word from a SimpleCommand.
fn get_cmd_name(cmd: &bast::SimpleCommand) -> Option<&bast::Word> {
    cmd.word_or_name.as_ref()
}

/// Helper: extract argument words from a SimpleCommand suffix.
fn get_arg_words(cmd: &bast::SimpleCommand) -> Vec<bast::Word> {
    let mut args = Vec::new();
    if let Some(ref suffix) = cmd.suffix {
        for item in &suffix.0 {
            if let bast::CommandPrefixOrSuffixItem::Word(w) = item {
                args.push(w.clone());
            }
        }
    }
    args
}

/// Helper: extract prefix assignment items from a SimpleCommand.
fn get_prefix_assignments(cmd: &bast::SimpleCommand) -> Vec<bast::CommandPrefixOrSuffixItem> {
    let mut items = Vec::new();
    if let Some(ref prefix) = cmd.prefix {
        for item in &prefix.0 {
            if matches!(item, bast::CommandPrefixOrSuffixItem::AssignmentWord(..)) {
                items.push(item.clone());
            }
        }
    }
    items
}

/// Helper: extract redirect items from a SimpleCommand (both prefix and suffix).
fn get_redirect_items(cmd: &bast::SimpleCommand) -> Vec<bast::CommandPrefixOrSuffixItem> {
    let mut items = Vec::new();
    if let Some(ref prefix) = cmd.prefix {
        for item in &prefix.0 {
            if matches!(item, bast::CommandPrefixOrSuffixItem::IoRedirect(_)) {
                items.push(item.clone());
            }
        }
    }
    if let Some(ref suffix) = cmd.suffix {
        for item in &suffix.0 {
            if matches!(item, bast::CommandPrefixOrSuffixItem::IoRedirect(_)) {
                items.push(item.clone());
            }
        }
    }
    items
}

/// Helper: build a SimpleCommand from parts.
fn build_simple_command(
    name: Option<bast::Word>,
    args: Vec<bast::Word>,
    assignments: Vec<bast::CommandPrefixOrSuffixItem>,
    redirects: Vec<bast::CommandPrefixOrSuffixItem>,
) -> bast::SimpleCommand {
    // Build prefix from assignments + prefix redirects
    let prefix = if assignments.is_empty() {
        None
    } else {
        Some(bast::CommandPrefix(assignments))
    };

    // Build suffix from args + suffix redirects
    let mut suffix_items: Vec<bast::CommandPrefixOrSuffixItem> = args
        .into_iter()
        .map(bast::CommandPrefixOrSuffixItem::Word)
        .collect();
    suffix_items.extend(redirects);

    let suffix = if suffix_items.is_empty() {
        None
    } else {
        Some(bast::CommandSuffix(suffix_items))
    };

    bast::SimpleCommand {
        prefix,
        word_or_name: name,
        suffix,
    }
}

/// Result of alias expansion
#[derive(Debug, Clone)]
pub enum AliasExpansionResult {
    /// No expansion occurred, return original node
    NoExpansion,
    /// Expansion succeeded, return new node
    Expanded(bast::SimpleCommand),
    /// Expansion resulted in a complex command (multiple statements/pipelines)
    /// that needs to be executed as a script
    ComplexAlias(String),
    /// Parse error during expansion
    ParseError(String),
}

/// Expand alias in a SimpleCommand if applicable.
/// Returns the expansion result.
pub fn expand_alias(
    ctx: &AliasExpansionContext,
    node: &bast::SimpleCommand,
    alias_expansion_stack: &mut HashSet<String>,
) -> AliasExpansionResult {
    // Need a command name to expand
    let name_word = match get_cmd_name(node) {
        Some(n) => n,
        None => return AliasExpansionResult::NoExpansion,
    };

    // Check if the command name is a literal unquoted word
    if !is_literal_unquoted_word(name_word) {
        return AliasExpansionResult::NoExpansion;
    }

    let cmd_name = match get_literal_value(name_word) {
        Some(n) => n,
        None => return AliasExpansionResult::NoExpansion,
    };

    // Check for alias
    let alias_value = match get_alias(ctx, cmd_name) {
        Some(v) => v.to_string(),
        None => return AliasExpansionResult::NoExpansion,
    };

    // Prevent infinite recursion
    if alias_expansion_stack.contains(cmd_name) {
        return AliasExpansionResult::NoExpansion;
    }

    alias_expansion_stack.insert(cmd_name.to_string());

    // Build the full command line: alias value + original args
    let mut full_command = alias_value.clone();

    // Check if alias value ends with a space (triggers expansion of next word)
    let expand_next = alias_value.ends_with(' ');

    // If not expanding next, append args directly
    if !expand_next {
        let orig_args = get_arg_words(node);
        for arg in &orig_args {
            full_command.push(' ');
            full_command.push_str(&arg.value);
        }
    }

    // Parse the expanded command using brush_parser
    let expanded_ast = match parse_command(&full_command) {
        Ok(ast) => ast,
        Err(e) => {
            alias_expansion_stack.remove(cmd_name);
            return AliasExpansionResult::ParseError(e);
        }
    };

    // Check if we got a single simple command
    // Program -> complete_commands (Vec<CompoundList>)
    // CompoundList -> Vec<CompoundListItem>
    // CompoundListItem -> (AndOrList, SeparatorOperator)
    // AndOrList -> first: Pipeline, additional: Vec<AndOr>
    // Pipeline -> seq: Vec<Command>
    if expanded_ast.complete_commands.len() != 1 {
        alias_expansion_stack.remove(cmd_name);
        return AliasExpansionResult::ComplexAlias(full_command);
    }

    let compound_list = &expanded_ast.complete_commands[0];
    if compound_list.0.len() != 1 {
        alias_expansion_stack.remove(cmd_name);
        return AliasExpansionResult::ComplexAlias(full_command);
    }

    let and_or_list = &compound_list.0[0].0;
    if !and_or_list.additional.is_empty() || and_or_list.first.seq.len() != 1 {
        alias_expansion_stack.remove(cmd_name);
        return AliasExpansionResult::ComplexAlias(full_command);
    }

    let expanded_cmd = &and_or_list.first.seq[0];
    match expanded_cmd {
        bast::Command::Simple(simple_cmd) => {
            // Merge the expanded command with original node's context
            let expanded_name = simple_cmd.word_or_name.clone();
            let mut expanded_args = get_arg_words(simple_cmd);

            // Merge assignments: original prefix assignments + expanded prefix assignments
            let mut merged_assignments = get_prefix_assignments(node);
            merged_assignments.extend(get_prefix_assignments(simple_cmd));

            // Merge redirections: expanded redirections + original redirections
            let mut merged_redirects = get_redirect_items(simple_cmd);
            merged_redirects.extend(get_redirect_items(node));

            // If alias ends with space, expand next word too (recursive alias on first arg)
            if expand_next {
                let orig_args = get_arg_words(node);
                if !orig_args.is_empty() {
                    // Add the original args to the expanded command's args
                    expanded_args.extend(orig_args);

                    // Now recursively expand the first arg if it's an alias
                    let first_arg = &expanded_args[0];
                    if is_literal_unquoted_word(first_arg) {
                        if let Some(first_arg_name) = get_literal_value(first_arg) {
                            if has_alias(ctx, first_arg_name) {
                                // Create a temporary node with the first arg as command
                                let temp_args: Vec<bast::Word> = expanded_args[1..].to_vec();
                                let temp_node = build_simple_command(
                                    Some(expanded_args[0].clone()),
                                    temp_args,
                                    vec![],
                                    vec![],
                                );
                                let expanded_first =
                                    expand_alias(ctx, &temp_node, alias_expansion_stack);
                                match expanded_first {
                                    AliasExpansionResult::Expanded(exp) => {
                                        let exp_args = get_arg_words(&exp);
                                        let new_node = build_simple_command(
                                            exp.word_or_name,
                                            exp_args,
                                            merged_assignments,
                                            merged_redirects,
                                        );
                                        return AliasExpansionResult::Expanded(new_node);
                                    }
                                    _ => {
                                        // Keep the original if expansion failed or was complex
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let new_node = build_simple_command(
                expanded_name,
                expanded_args,
                merged_assignments,
                merged_redirects,
            );
            AliasExpansionResult::Expanded(new_node)
        }
        _ => {
            // Alias expanded to a compound command
            alias_expansion_stack.remove(cmd_name);
            AliasExpansionResult::ComplexAlias(full_command)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_env() -> HashMap<String, String> {
        HashMap::new()
    }

    fn make_word(value: &str) -> bast::Word {
        bast::Word {
            value: value.to_string(),
            loc: None,
        }
    }

    fn make_simple_cmd(name: &str, args: &[&str]) -> bast::SimpleCommand {
        let arg_words: Vec<bast::Word> = args.iter().map(|a| make_word(a)).collect();
        build_simple_command(Some(make_word(name)), arg_words, vec![], vec![])
    }

    #[test]
    fn test_is_literal_unquoted_word() {
        let word = make_word("echo");
        assert!(is_literal_unquoted_word(&word));

        // Word with quotes - not literal
        let word = make_word("\"echo\"");
        assert!(!is_literal_unquoted_word(&word));

        // Word with expansion
        let word = make_word("$var");
        assert!(!is_literal_unquoted_word(&word));
    }

    #[test]
    fn test_get_literal_value() {
        let word = make_word("hello");
        assert_eq!(get_literal_value(&word), Some("hello"));

        let word = make_word("\"hello\"");
        assert_eq!(get_literal_value(&word), None);

        let word = make_word("");
        assert_eq!(get_literal_value(&word), None);
    }

    #[test]
    fn test_set_get_alias() {
        let mut env = make_env();
        set_alias(&mut env, "ll", "ls -la");

        let ctx = AliasExpansionContext { env: &env };
        assert_eq!(get_alias(&ctx, "ll"), Some("ls -la"));
        assert_eq!(get_alias(&ctx, "nonexistent"), None);
    }

    #[test]
    fn test_unset_alias() {
        let mut env = make_env();
        set_alias(&mut env, "ll", "ls -la");

        assert!(unset_alias(&mut env, "ll"));
        assert!(!unset_alias(&mut env, "ll")); // Already removed

        let ctx = AliasExpansionContext { env: &env };
        assert_eq!(get_alias(&ctx, "ll"), None);
    }

    #[test]
    fn test_get_all_aliases() {
        let mut env = make_env();
        set_alias(&mut env, "ll", "ls -la");
        set_alias(&mut env, "la", "ls -a");

        let aliases = get_all_aliases(&env);
        assert_eq!(aliases.len(), 2);
    }

    #[test]
    fn test_expand_alias_no_alias() {
        let env = make_env();
        let ctx = AliasExpansionContext { env: &env };

        let node = make_simple_cmd("echo", &["hello"]);

        let mut stack = HashSet::new();
        let result = expand_alias(&ctx, &node, &mut stack);
        assert!(matches!(result, AliasExpansionResult::NoExpansion));
    }

    #[test]
    fn test_expand_alias_simple() {
        let mut env = make_env();
        set_alias(&mut env, "ll", "ls -la");
        let ctx = AliasExpansionContext { env: &env };

        let node = make_simple_cmd("ll", &[]);

        let mut stack = HashSet::new();
        let result = expand_alias(&ctx, &node, &mut stack);

        match result {
            AliasExpansionResult::Expanded(expanded) => {
                let cmd_name = expanded.word_or_name.as_ref().and_then(|w| get_literal_value(w));
                assert_eq!(cmd_name, Some("ls"));
            }
            _ => panic!("Expected Expanded result"),
        }
    }

    #[test]
    fn test_expand_alias_prevents_recursion() {
        let mut env = make_env();
        // Create a self-referencing alias
        set_alias(&mut env, "foo", "foo bar");
        let ctx = AliasExpansionContext { env: &env };

        let node = make_simple_cmd("foo", &[]);

        let mut stack = HashSet::new();
        stack.insert("foo".to_string()); // Simulate already expanding foo

        let result = expand_alias(&ctx, &node, &mut stack);
        assert!(matches!(result, AliasExpansionResult::NoExpansion));
    }
}
