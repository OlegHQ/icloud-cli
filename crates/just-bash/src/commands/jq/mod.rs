use crate::commands::jaq_support::{
    collect_inputs, compile_filter, format_values, last_truthy, run_filter,
};
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;
use jaq_all::fmts::Format;

pub struct JqCommand;

struct JqOptions {
    raw: bool,
    compact: bool,
    exit_status: bool,
    slurp: bool,
    null_input: bool,
    join_output: bool,
    sort_keys: bool,
    use_tab: bool,
    filter: String,
    files: Vec<String>,
}

fn parse_jq_args(args: &[String]) -> Result<JqOptions, CommandResult> {
    let mut options = JqOptions {
        raw: false,
        compact: false,
        exit_status: false,
        slurp: false,
        null_input: false,
        join_output: false,
        sort_keys: false,
        use_tab: false,
        filter: ".".to_string(),
        files: Vec::new(),
    };
    let mut filter_set = false;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-r" | "--raw-output" => options.raw = true,
            "-c" | "--compact-output" => options.compact = true,
            "-e" | "--exit-status" => options.exit_status = true,
            "-s" | "--slurp" => options.slurp = true,
            "-n" | "--null-input" => options.null_input = true,
            "-j" | "--join-output" => options.join_output = true,
            "-S" | "--sort-keys" => options.sort_keys = true,
            "--tab" => options.use_tab = true,
            "-a" | "--ascii" | "-C" | "--color" | "-M" | "--monochrome" => {}
            "-" => options.files.push("-".to_string()),
            _ if arg.starts_with("--") => {
                return Err(CommandResult::with_exit_code(
                    String::new(),
                    format!("jq: Unknown option: {arg}\n"),
                    2,
                ));
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                for flag in arg[1..].chars() {
                    match flag {
                        'r' => options.raw = true,
                        'c' => options.compact = true,
                        'e' => options.exit_status = true,
                        's' => options.slurp = true,
                        'n' => options.null_input = true,
                        'j' => options.join_output = true,
                        'S' => options.sort_keys = true,
                        'a' | 'C' | 'M' => {}
                        _ => {
                            return Err(CommandResult::with_exit_code(
                                String::new(),
                                format!("jq: Unknown option: -{flag}\n"),
                                2,
                            ));
                        }
                    }
                }
            }
            _ if !filter_set => {
                options.filter = arg.clone();
                filter_set = true;
            }
            _ => options.files.push(arg.clone()),
        }
        i += 1;
    }

    Ok(options)
}

const JQ_HELP: &str = "\
Usage: jq [OPTIONS] FILTER [FILE]

command-line JSON processor powered by jaq

Options:
  -r, --raw-output   output strings without quotes
  -c, --compact-output
                     compact output
  -e, --exit-status  set exit status based on the last output value
  -s, --slurp        read all input values into one array
  -n, --null-input   run the filter with null input
  -j, --join-output  suppress separators between outputs
  -S, --sort-keys    sort object keys
      --tab          indent with tabs
      --help         display this help and exit
";

async fn load_inputs(ctx: &CommandContext, files: &[String]) -> Result<Vec<String>, CommandResult> {
    if files.is_empty() || (files.len() == 1 && files[0] == "-") {
        return Ok(vec![ctx.stdin.clone()]);
    }

    let mut contents = Vec::new();
    for file in files {
        if file == "-" {
            contents.push(ctx.stdin.clone());
            continue;
        }

        let path = ctx.fs.resolve_path(&ctx.cwd, file);
        match ctx.fs.read_file(&path).await {
            Ok(content) => contents.push(content),
            Err(_) => {
                return Err(CommandResult::with_exit_code(
                    String::new(),
                    format!("jq: {file}: No such file or directory\n"),
                    2,
                ));
            }
        }
    }

    Ok(contents)
}

#[async_trait]
impl Command for JqCommand {
    fn name(&self) -> &'static str {
        "jq"
    }

    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        if ctx.args.iter().any(|arg| arg == "--help") {
            return CommandResult::success(JQ_HELP.to_string());
        }

        let options = match parse_jq_args(&ctx.args) {
            Ok(options) => options,
            Err(result) => return result,
        };

        let filter = match compile_filter(&options.filter) {
            Ok(filter) => filter,
            Err(err) => {
                return CommandResult::with_exit_code(
                    String::new(),
                    format!("jq: {}\n", err.message.trim_end()),
                    err.exit_code,
                );
            }
        };

        let contents = match load_inputs(&ctx, &options.files).await {
            Ok(contents) => contents,
            Err(result) => return result,
        };

        let inputs = if options.null_input {
            Vec::new()
        } else {
            match collect_inputs(Format::Json, &contents, options.slurp) {
                Ok(inputs) => inputs,
                Err(err) => {
                    return CommandResult::with_exit_code(
                        String::new(),
                        format!("jq: parse error: {err}\n"),
                        5,
                    );
                }
            }
        };

        let values = match run_filter(&filter, options.null_input, inputs) {
            Ok(values) => values,
            Err(err) => {
                return CommandResult::with_exit_code(
                    String::new(),
                    format!("jq: error: {err}\n"),
                    5,
                );
            }
        };

        let format = if options.raw {
            Format::Raw
        } else {
            Format::Json
        };
        let stdout = match format_values(
            &values,
            format,
            options.compact,
            options.join_output,
            options.sort_keys,
            2,
            options.use_tab,
        ) {
            Ok(stdout) => stdout,
            Err(err) => {
                return CommandResult::with_exit_code(
                    String::new(),
                    format!("jq: error: {err}\n"),
                    5,
                );
            }
        };

        let exit_code =
            if options.exit_status && (values.is_empty() || last_truthy(&values) != Some(true)) {
                1
            } else {
                0
            };

        CommandResult::with_exit_code(stdout, String::new(), exit_code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_utils;
    use crate::fs::FileSystem;

    fn make_ctx(args: &[&str], stdin: &str) -> CommandContext {
        test_utils::make_ctx_with_stdin(args.to_vec(), stdin)
    }

    async fn make_ctx_with_files(args: &[&str], stdin: &str, files: &[(&str, &str)]) -> CommandContext {
        test_utils::make_ctx_with_stdin_and_files(args.to_vec(), stdin, files.to_vec()).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_identity() {
        let result = JqCommand.execute(make_ctx(&["."], r#"{"a":1}"#)).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("\"a\": 1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_raw_output() {
        let result = JqCommand
            .execute(make_ctx(&["-r", ".name"], r#"{"name":"hello"}"#))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_compact_sorted_output() {
        let result = JqCommand
            .execute(make_ctx(&["-Sc", "."], r#"{"b":2,"a":1}"#))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "{\"a\":1,\"b\":2}\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_null_input() {
        let result = JqCommand.execute(make_ctx(&["-n", "1 + 2"], "")).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "3\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_slurp() {
        let result = JqCommand.execute(make_ctx(&["-sc", "."], "1\n2\n3")).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "[1,2,3]\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_join_output() {
        let result = JqCommand.execute(make_ctx(&["-j", ".[]"], "[1,2,3]")).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "123");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_tab_indent() {
        let result = JqCommand
            .execute(make_ctx(&["--tab", "."], r#"{"a":1}"#))
            .await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("\t\"a\""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_json_stream() {
        let result = JqCommand
            .execute(make_ctx(&[".value"], r#"{"value":1}{"value":2}"#))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "1\n2\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_map_and_select() {
        let result = JqCommand
            .execute(make_ctx(&["[.[] | select(. > 2) | . * 2]"], "[1,2,3,4]"))
            .await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("6"));
        assert!(result.stdout.contains("8"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_reduce() {
        let result = JqCommand
            .execute(make_ctx(&["reduce .[] as $x (0; . + $x)"], "[1,2,3]"))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "6\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_file_input() {
        let ctx = make_ctx_with_files(
            &[".", "/data.json"],
            "",
            &[("/data.json", r#"{"key":"value"}"#)],
        )
        .await;
        let result = JqCommand.execute(ctx).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("\"key\""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_missing_file() {
        let result = JqCommand
            .execute(make_ctx(&[".", "/missing.json"], ""))
            .await;
        assert_eq!(result.exit_code, 2);
        assert!(result.stderr.contains("No such file"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_invalid_json() {
        let result = JqCommand.execute(make_ctx(&["."], "not json")).await;
        assert_eq!(result.exit_code, 5);
        assert!(result.stderr.contains("parse error"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_unknown_function() {
        let result = JqCommand.execute(make_ctx(&["foo"], "{}")).await;
        assert_eq!(result.exit_code, 3);
        assert!(result.stderr.contains("undefined"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jq_exit_status_falsey() {
        let result = JqCommand
            .execute(make_ctx(&["-e", ".missing"], r#"{"a":1}"#))
            .await;
        assert_eq!(result.exit_code, 1);
    }
}
