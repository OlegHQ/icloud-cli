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
mod tests {
    use super::*;
    use crate::commands::test_utils::*;
    use crate::fs::FileSystem;

    fn make_ctx(args: Vec<&str>, stdin: &str) -> CommandContext {
        make_ctx_with_stdin(args, stdin)
    }

    fn make_ctx_with_fs(args: Vec<&str>, stdin: &str, fs: Arc<InMemoryFs>) -> CommandContext {
        make_ctx_with_stdin_and_fs(args, stdin, fs)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_basic_substitution() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/world/rust/"], "hello world\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "hello rust\n");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_global_substitution() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/o/0/g"], "foo boo\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "f00 b00\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_silent_mode_with_print() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-n", "2p"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "b\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_line_range_delete() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["1,2d"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "c\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_pattern_match_delete() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["/foo/d"], "foo\nbar\nfoo\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "bar\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_multiple_expressions() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-e", "s/a/x/", "-e", "s/b/y/"], "ab\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "xy\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_in_place_editing() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/test.txt", b"old content\n").await.unwrap();
        let cmd = SedCommand;
        let ctx = make_ctx_with_fs(vec!["-i", "s/old/new/", "/test.txt"], "", fs.clone());
        let result = cmd.execute(ctx).await;
        assert_eq!(result.exit_code, 0);
        let content = fs.read_file("/test.txt").await.unwrap();
        assert_eq!(content, "new content\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_extended_regex() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-E", "s/[0-9]+/NUM/"], "abc123\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "abcNUM\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_hold_space_workflow() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-n", "1h;2{H;g;p}"], "a\nb\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\nb\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_append_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["2a\\ new"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\nb\nnew\nc\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_insert_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["2i\\ new"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\nnew\nb\nc\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_change_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["2c\\ new"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\nnew\nc\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_quit_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["2q"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\nb\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_quit_silent_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["2Q"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_step_address() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-n", "0~2p"], "a\nb\nc\nd\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "b\nd\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_transliterate() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["y/abc/ABC/"], "abc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "ABC\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_delete_first_line_cycle() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["N;P;D"], "1\n2\n3\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "1\n2\n3\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_empty_script() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec![""], "hello\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "hello\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_no_script_error() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec![], "hello\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.exit_code, 1);
        assert!(result.stderr.contains("no script"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_file_not_found() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/a/b/", "nonexistent"], "");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.exit_code, 1);
        assert!(result.stderr.contains("No such file"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_case_insensitive() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/HELLO/hi/i"], "Hello\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "hi\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_nth_occurrence() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/a/X/2"], "aaa\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "aXa\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_backreferences() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/\\(hel\\)\\(lo\\)/\\2\\1/"], "hello\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "lohel\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ampersand_replacement() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/world/[&]/"], "hello world\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "hello [world]\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_negated_address() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["2!d"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "b\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_pattern_range() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["/start/,/end/d"], "a\nstart\nmid\nend\nb\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\nb\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_last_line_address() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["$d"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\nb\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_list_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-n", "l"], "a\tb\n");
        let result = cmd.execute(ctx).await;
        assert!(result.stdout.contains("\\t"));
        assert!(result.stdout.contains("$"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_line_number_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["="], "a\nb\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "1\na\n2\nb\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_zap_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["z"], "hello\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_print_first_line() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-n", "N;P"], "a\nb\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "a\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_grouped_commands() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-n", "2{ p; = }"], "a\nb\nc\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "b\n2\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_substitution_tracking_t() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/a/x/;t end;d;:end"], "abc\nxyz\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "xbc\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[allow(non_snake_case)]
    async fn test_substitution_tracking_T() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/a/x/;T end;p;:end"], "abc\nxyz\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "xbc\nxbc\nxyz\n");
    }

    /// sed-rs uses the real filesystem for `r` — cannot read from InMemoryFs.
    /// See KNOWN_ISSUES.md.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn test_read_file_command() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/append.txt", b"appended").await.unwrap();
        let cmd = SedCommand;
        let ctx = make_ctx_with_fs(vec!["r /append.txt"], "line\n", fs);
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "line\nappended\n");
    }

    /// sed-rs uses the real filesystem for `w` — cannot write to InMemoryFs.
    /// See KNOWN_ISSUES.md.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn test_write_file_command() {
        let fs = Arc::new(InMemoryFs::new());
        let cmd = SedCommand;
        let ctx = make_ctx_with_fs(vec!["w /output.txt"], "hello\n", fs.clone());
        let result = cmd.execute(ctx).await;
        assert_eq!(result.exit_code, 0);
        let content = fs.read_file("/output.txt").await.unwrap();
        assert_eq!(content, "hello\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stdin_marker() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/a/b/", "-"], "aaa\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "baa\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_multiple_files() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/file1.txt", b"a\n").await.unwrap();
        fs.write_file("/file2.txt", b"b\n").await.unwrap();
        let cmd = SedCommand;
        let ctx = make_ctx_with_fs(vec!["s/./X/", "/file1.txt", "/file2.txt"], "", fs);
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "X\nX\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_bre_mode() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/\\(foo\\)/[\\1]/"], "foo\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "[foo]\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ere_mode() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-E", "s/(foo)/[\\1]/"], "foo\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "[foo]\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_posix_classes() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["s/[[:digit:]]/X/g"], "a1b2\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "aXbX\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_help_flag() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["--help"], "");
        let result = cmd.execute(ctx).await;
        assert!(result.stdout.contains("Usage:"));
        assert!(result.stdout.contains("sed"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_script_file() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/script.sed", b"s/old/new/").await.unwrap();
        let cmd = SedCommand;
        let ctx = make_ctx_with_fs(vec!["-f", "/script.sed"], "old text\n", fs);
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "new text\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_branch_to_end() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["/foo/b;d"], "foo\nbar\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "foo\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_exchange_command() {
        let cmd = SedCommand;
        let ctx = make_ctx(vec!["-n", "x;p"], "a\nb\n");
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "\na\n");
    }
}
