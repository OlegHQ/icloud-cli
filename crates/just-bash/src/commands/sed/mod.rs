pub mod regex_utils;

use self::regex_utils::preprocess_bre_script;
use crate::commands::errors::no_such_file;
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;
use std::sync::Arc;

pub struct SedCommand;

#[async_trait]
impl Command for SedCommand {
    fn name(&self) -> &'static str {
        "sed"
    }

    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        if ctx.args.iter().any(|a| a == "--help") {
            return CommandResult::success(
                "Usage: sed [OPTION]... {script} [input-file]...\n\n\
                 Stream editor for filtering and transforming text.\n\n\
                 Options:\n  \
                 -n, --quiet, --silent  suppress automatic printing of pattern space\n  \
                 -e script              add the script to commands to be executed\n  \
                 -f script-file         read script from file\n  \
                 -i, --in-place         edit files in place\n  \
                 -E, -r, --regexp-extended  use extended regular expressions\n      \
                 --help             display this help and exit\n"
                    .to_string(),
            );
        }

        let mut scripts: Vec<String> = Vec::new();
        let mut script_files: Vec<String> = Vec::new();
        let mut silent = false;
        let mut in_place = false;
        let mut extended_regex = false;
        let mut files: Vec<String> = Vec::new();

        let args = &ctx.args;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-n" || arg == "--quiet" || arg == "--silent" {
                silent = true;
            } else if arg == "-i" || arg == "--in-place" {
                in_place = true;
            } else if arg.starts_with("-i") && arg.len() > 2 {
                in_place = true;
            } else if arg == "-E" || arg == "-r" || arg == "--regexp-extended" {
                extended_regex = true;
            } else if arg == "-e" {
                if i + 1 < args.len() {
                    i += 1;
                    scripts.push(args[i].clone());
                }
            } else if arg == "-f" {
                if i + 1 < args.len() {
                    i += 1;
                    script_files.push(args[i].clone());
                }
            } else if arg.starts_with("--") {
                return CommandResult::error(format!("sed: unknown option: {}\n", arg));
            } else if arg == "-" {
                files.push(arg.clone());
            } else if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut needs_next_arg = false;
                let mut next_is_for = ' ';
                for &c in &chars {
                    match c {
                        'n' => silent = true,
                        'i' => in_place = true,
                        'E' | 'r' => extended_regex = true,
                        'e' => {
                            needs_next_arg = true;
                            next_is_for = 'e';
                        }
                        'f' => {
                            needs_next_arg = true;
                            next_is_for = 'f';
                        }
                        _ => {
                            return CommandResult::error(format!(
                                "sed: unknown option: -{}\n",
                                c
                            ));
                        }
                    }
                }
                if needs_next_arg && i + 1 < args.len() {
                    i += 1;
                    if next_is_for == 'e' {
                        scripts.push(args[i].clone());
                    } else {
                        script_files.push(args[i].clone());
                    }
                }
            } else if scripts.is_empty() && script_files.is_empty() {
                scripts.push(arg.clone());
            } else {
                files.push(arg.clone());
            }
            i += 1;
        }

        // Read scripts from -f files
        for script_file in &script_files {
            let script_path = ctx.fs.resolve_path(&ctx.cwd, script_file);
            match ctx.fs.read_file(&script_path).await {
                Ok(content) => {
                    for line in content.split('\n') {
                        let trimmed = line.trim();
                        if !trimmed.is_empty() && !trimmed.starts_with('#') {
                            scripts.push(trimmed.to_string());
                        }
                    }
                }
                Err(_) => {
                    return CommandResult::error(format!(
                        "sed: couldn't open file {}: No such file or directory\n",
                        script_file
                    ));
                }
            }
        }

        if scripts.is_empty() {
            return CommandResult::error("sed: no script specified\n".to_string());
        }

        // Check for #n (silent mode) in first script
        if scripts.first().map_or(false, |s| s.starts_with("#n")) {
            silent = true;
        }

        // Join all scripts and convert BRE→ERE when not in extended mode
        let full_script = scripts.join("\n");
        let processed_script = if extended_regex {
            full_script
        } else {
            preprocess_bre_script(&full_script)
        };

        // Build sed-rs engine
        let sed = match sed_rs::Sed::new(&processed_script) {
            Ok(mut s) => {
                s.quiet(silent);
                s
            }
            Err(e) => return CommandResult::error(format!("sed: {}\n", e)),
        };

        if in_place {
            if files.is_empty() {
                return CommandResult::error(
                    "sed: -i requires at least one file argument\n".to_string(),
                );
            }
            for file in &files {
                if file == "-" {
                    continue;
                }
                let file_path = ctx.fs.resolve_path(&ctx.cwd, file);
                match ctx.fs.read_file(&file_path).await {
                    Ok(file_content) => match sed.eval(&file_content) {
                        Ok(output) => {
                            let _ = ctx.fs.write_file(&file_path, output.as_bytes()).await;
                        }
                        Err(e) => {
                            return CommandResult::error(format!("sed: {}\n", e));
                        }
                    },
                    Err(_) => {
                        return CommandResult::error(no_such_file("sed", file));
                    }
                }
            }
            return CommandResult::success(String::new());
        }

        // Collect input content
        let content = if files.is_empty() {
            ctx.stdin.clone()
        } else {
            let mut content = String::new();
            let mut stdin_consumed = false;
            for file in &files {
                let file_content = if file == "-" {
                    if stdin_consumed {
                        String::new()
                    } else {
                        stdin_consumed = true;
                        ctx.stdin.clone()
                    }
                } else {
                    let file_path = ctx.fs.resolve_path(&ctx.cwd, file);
                    match ctx.fs.read_file(&file_path).await {
                        Ok(c) => c,
                        Err(_) => {
                            return CommandResult::error(no_such_file("sed", file));
                        }
                    }
                };
                if !content.is_empty() && !file_content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str(&file_content);
            }
            content
        };

        match sed.eval(&content) {
            Ok(output) => CommandResult::success(output),
            Err(e) => CommandResult::error(format!("sed: {}\n", e)),
        }
    }
}

#[cfg(test)]
mod tests;
