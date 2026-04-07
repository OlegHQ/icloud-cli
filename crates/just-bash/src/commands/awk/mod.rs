// src/commands/awk/mod.rs

use crate::commands::errors::no_such_file;
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;
use std::io::BufReader;

pub struct AwkCommand;

/// Process escape sequences in a string (for -F and -v options).
fn process_escapes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => result.push('\t'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('b') => result.push('\x08'),
                Some('f') => result.push('\x0c'),
                Some('a') => result.push('\x07'),
                Some('v') => result.push('\x0b'),
                Some('\\') => result.push('\\'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[async_trait]
impl Command for AwkCommand {
    fn name(&self) -> &'static str {
        "awk"
    }

    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        // Handle --help
        if ctx.args.iter().any(|a| a == "--help") {
            return CommandResult::success(
                "Usage: awk [OPTIONS] 'PROGRAM' [FILE...]\n\n\
                 Pattern scanning and text processing language.\n\n\
                 Options:\n  \
                 -F FS      use FS as field separator\n  \
                 -v VAR=VAL assign VAL to variable VAR\n      \
                 --help     display this help and exit\n"
                    .to_string(),
            );
        }

        let mut field_sep: Option<String> = None;
        let mut preset_vars: Vec<(String, String)> = Vec::new();
        let mut program_idx: Option<usize> = None;

        // Parse options
        let args = &ctx.args;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-F" && i + 1 < args.len() {
                i += 1;
                field_sep = Some(process_escapes(&args[i]));
                i += 1;
            } else if arg.starts_with("-F") && arg.len() > 2 {
                field_sep = Some(process_escapes(&arg[2..]));
                i += 1;
            } else if arg == "-v" && i + 1 < args.len() {
                i += 1;
                let assignment = &args[i];
                if let Some(eq_idx) = assignment.find('=') {
                    let var_name = assignment[..eq_idx].to_string();
                    let var_value = process_escapes(&assignment[eq_idx + 1..]);
                    preset_vars.push((var_name, var_value));
                }
                i += 1;
            } else if arg.starts_with("--") {
                return CommandResult::error(format!("awk: unknown option: {}\n", arg));
            } else if arg.starts_with('-') && arg.len() > 1 {
                let opt_char = arg.chars().nth(1).unwrap();
                if opt_char != 'F' && opt_char != 'v' {
                    return CommandResult::error(format!("awk: unknown option: -{}\n", opt_char));
                }
                i += 1;
            } else {
                program_idx = Some(i);
                break;
            }
        }

        let program_idx = match program_idx {
            Some(idx) => idx,
            None => {
                return CommandResult::error("awk: missing program\n".to_string());
            }
        };

        let program_text = &args[program_idx];
        let files: Vec<String> = args[program_idx + 1..].to_vec();

        // Parse the AWK program using awk-rs
        let mut lexer = awk_rs::Lexer::new(program_text);
        let tokens = match lexer.tokenize() {
            Ok(t) => t,
            Err(e) => return CommandResult::error(format!("awk: {}\n", e)),
        };
        let mut parser = awk_rs::Parser::new(tokens);
        let program = match parser.parse() {
            Ok(p) => p,
            Err(e) => return CommandResult::error(format!("awk: {}\n", e)),
        };

        // Create interpreter
        let mut interp = awk_rs::Interpreter::new(&program);

        // Configure field separator
        if let Some(ref fs) = field_sep {
            interp.set_fs(fs);
        }

        // Set preset variables
        for (name, value) in &preset_vars {
            interp.set_variable(name, value);
        }

        // Set up ARGC/ARGV
        let mut argv = vec!["awk".to_string()];
        argv.extend(files.iter().cloned());
        interp.set_args(argv);

        // Collect inputs
        let mut input_bufs: Vec<Vec<u8>> = Vec::new();

        if !files.is_empty() {
            for file in &files {
                if file == "-" {
                    input_bufs.push(ctx.stdin.as_bytes().to_vec());
                } else {
                    let file_path = ctx.fs.resolve_path(&ctx.cwd, file);
                    match ctx.fs.read_file(&file_path).await {
                        Ok(content) => input_bufs.push(content.into_bytes()),
                        Err(_) => return CommandResult::error(no_such_file("awk", file)),
                    }
                }
            }
        } else {
            input_bufs.push(ctx.stdin.as_bytes().to_vec());
        }

        // Run the program
        let mut output = Vec::new();
        let inputs: Vec<BufReader<&[u8]>> = input_bufs
            .iter()
            .map(|buf| BufReader::new(buf.as_slice()))
            .collect();

        let exit_code = match interp.run(inputs, &mut output) {
            Ok(code) => code,
            Err(e) => return CommandResult::error(format!("awk: {}\n", e)),
        };

        let stdout = String::from_utf8_lossy(&output).to_string();
        CommandResult::with_exit_code(stdout, String::new(), exit_code)
    }
}

#[cfg(test)]
mod tests;
