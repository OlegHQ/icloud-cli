use crate::commands::jaq_support::{
    collect_inputs, compile_filter, format_values, last_truthy, run_filter,
};
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;
use jaq_all::fmts::Format;
use std::path::Path;

pub struct YqCommand;

struct YqOptions {
    input_format: Option<Format>,
    output_format: Option<Format>,
    raw: bool,
    compact: bool,
    exit_status: bool,
    slurp: bool,
    null_input: bool,
    join_output: bool,
    sort_keys: bool,
    use_tab: bool,
    indent: usize,
    inplace: bool,
    filter: String,
    files: Vec<String>,
}

struct InputSource {
    name: String,
    content: String,
    format: Format,
}

fn parse_data_format(value: &str) -> Result<Format, String> {
    match value {
        "yaml" | "yml" | "y" => Ok(Format::Yaml),
        "json" | "j" => Ok(Format::Json),
        "toml" | "t" => Ok(Format::Toml),
        _ => Err(format!("Unknown format: {value}")),
    }
}

fn infer_input_format(path: &str) -> Format {
    match Format::determine(Path::new(path)) {
        Some(Format::Json) => Format::Json,
        Some(Format::Toml) => Format::Toml,
        _ => Format::Yaml,
    }
}

fn parse_yq_args(args: &[String]) -> Result<YqOptions, CommandResult> {
    let mut options = YqOptions {
        input_format: None,
        output_format: None,
        raw: false,
        compact: false,
        exit_status: false,
        slurp: false,
        null_input: false,
        join_output: false,
        sort_keys: false,
        use_tab: false,
        indent: 2,
        inplace: false,
        filter: ".".to_string(),
        files: Vec::new(),
    };
    let mut filter_set = false;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if let Some(value) = arg.strip_prefix("--input=") {
            options.input_format = Some(parse_data_format(value).map_err(|err| {
                CommandResult::with_exit_code(String::new(), format!("yq: {err}\n"), 2)
            })?);
            i += 1;
            continue;
        }

        if let Some(value) = arg.strip_prefix("--output=") {
            options.output_format = Some(parse_data_format(value).map_err(|err| {
                CommandResult::with_exit_code(String::new(), format!("yq: {err}\n"), 2)
            })?);
            i += 1;
            continue;
        }

        match arg.as_str() {
            "-p" | "--input-format" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    return Err(CommandResult::with_exit_code(
                        String::new(),
                        "yq: --input-format requires an argument\n".to_string(),
                        2,
                    ));
                };
                options.input_format = Some(parse_data_format(value).map_err(|err| {
                    CommandResult::with_exit_code(String::new(), format!("yq: {err}\n"), 2)
                })?);
            }
            "-o" | "--output-format" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    return Err(CommandResult::with_exit_code(
                        String::new(),
                        "yq: --output-format requires an argument\n".to_string(),
                        2,
                    ));
                };
                options.output_format = Some(parse_data_format(value).map_err(|err| {
                    CommandResult::with_exit_code(String::new(), format!("yq: {err}\n"), 2)
                })?);
            }
            "-I" | "--indent" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    return Err(CommandResult::with_exit_code(
                        String::new(),
                        "yq: --indent requires an argument\n".to_string(),
                        2,
                    ));
                };
                options.indent = value.parse().unwrap_or(2);
            }
            "-J" => options.input_format = Some(Format::Json),
            "-T" => options.input_format = Some(Format::Toml),
            "-y" => options.output_format = Some(Format::Yaml),
            "-t" => options.output_format = Some(Format::Toml),
            "-r" | "--raw-output" => options.raw = true,
            "-c" | "--compact-output" => options.compact = true,
            "-e" | "--exit-status" => options.exit_status = true,
            "-s" | "--slurp" => options.slurp = true,
            "-n" | "--null-input" => options.null_input = true,
            "-j" | "--join-output" => options.join_output = true,
            "-S" | "--sort-keys" => options.sort_keys = true,
            "--tab" => options.use_tab = true,
            "-i" | "--inplace" => options.inplace = true,
            _ if arg.starts_with("-p") && arg.len() > 2 => {
                options.input_format = Some(parse_data_format(&arg[2..]).map_err(|err| {
                    CommandResult::with_exit_code(String::new(), format!("yq: {err}\n"), 2)
                })?);
            }
            _ if arg.starts_with("-o") && arg.len() > 2 => {
                options.output_format = Some(parse_data_format(&arg[2..]).map_err(|err| {
                    CommandResult::with_exit_code(String::new(), format!("yq: {err}\n"), 2)
                })?);
            }
            "-" => options.files.push("-".to_string()),
            _ if arg.starts_with("--") => {
                return Err(CommandResult::with_exit_code(
                    String::new(),
                    format!("yq: Unknown option: {arg}\n"),
                    2,
                ));
            }
            _ if arg.starts_with('-') && arg.len() > 1 => {
                for flag in arg[1..].chars() {
                    match flag {
                        'J' => options.input_format = Some(Format::Json),
                        'T' => options.input_format = Some(Format::Toml),
                        'y' => options.output_format = Some(Format::Yaml),
                        't' => options.output_format = Some(Format::Toml),
                        'r' => options.raw = true,
                        'c' => options.compact = true,
                        'e' => options.exit_status = true,
                        's' => options.slurp = true,
                        'n' => options.null_input = true,
                        'j' => options.join_output = true,
                        'S' => options.sort_keys = true,
                        'i' => options.inplace = true,
                        _ => {
                            return Err(CommandResult::with_exit_code(
                                String::new(),
                                format!("yq: Unknown option: -{flag}\n"),
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

const YQ_HELP: &str = "\
Usage: yq [OPTIONS] FILTER [FILE]

lq-style YAML/TOML/JSON query wrapper powered by jaq

Defaults:
  input  = yaml
  output = jq/json on stdout, input format for --inplace

Options:
  -J                 treat input as JSON
  -T                 treat input as TOML
  -p, --input-format FORMAT
                     input format: yaml, json, toml
  -y                 write YAML output
  -t                 write TOML output
  -o, --output-format FORMAT
                     output format: json, yaml, toml
  -i, --inplace      rewrite input files instead of writing to stdout
  -r, --raw-output   output strings without quotes
  -c, --compact-output
                     compact JSON output
  -e, --exit-status  set exit status based on the last output value
  -s, --slurp        read all input documents into one array
  -n, --null-input   run the filter with null input
  -j, --join-output  suppress separators between outputs
  -S, --sort-keys    sort object keys
  -I, --indent N     indentation width for pretty output
      --tab          indent with tabs
      --help         display this help and exit
";

async fn load_sources(
    ctx: &CommandContext,
    options: &YqOptions,
) -> Result<Vec<InputSource>, CommandResult> {
    if options.files.is_empty() || (options.files.len() == 1 && options.files[0] == "-") {
        return Ok(vec![InputSource {
            name: "stdin".to_string(),
            content: ctx.stdin.clone(),
            format: options.input_format.unwrap_or(Format::Yaml),
        }]);
    }

    let mut sources = Vec::new();
    for file in &options.files {
        if file == "-" {
            sources.push(InputSource {
                name: "stdin".to_string(),
                content: ctx.stdin.clone(),
                format: options.input_format.unwrap_or(Format::Yaml),
            });
            continue;
        }

        let path = ctx.fs.resolve_path(&ctx.cwd, file);
        match ctx.fs.read_file(&path).await {
            Ok(content) => sources.push(InputSource {
                name: file.clone(),
                content,
                format: options
                    .input_format
                    .unwrap_or_else(|| infer_input_format(file)),
            }),
            Err(_) => {
                return Err(CommandResult::with_exit_code(
                    String::new(),
                    format!("yq: {file}: No such file or directory\n"),
                    2,
                ));
            }
        }
    }

    Ok(sources)
}

fn resolve_output_format(options: &YqOptions, source_format: Format) -> Format {
    options.output_format.unwrap_or({
        if options.inplace {
            source_format
        } else {
            Format::Json
        }
    })
}

fn validate_output_mode(raw: bool, output_format: Format) -> Result<(), CommandResult> {
    if raw && !matches!(output_format, Format::Json) {
        return Err(CommandResult::with_exit_code(
            String::new(),
            "yq: --raw-output only works with jq/json output\n".to_string(),
            2,
        ));
    }
    Ok(())
}

fn collect_source_inputs(
    source: &InputSource,
    slurp: bool,
) -> Result<Vec<jaq_all::json::Val>, String> {
    collect_inputs(source.format, std::slice::from_ref(&source.content), slurp)
}

#[async_trait]
impl Command for YqCommand {
    fn name(&self) -> &'static str {
        "yq"
    }

    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        if ctx.args.iter().any(|arg| arg == "--help") {
            return CommandResult::success(YQ_HELP.to_string());
        }

        let options = match parse_yq_args(&ctx.args) {
            Ok(options) => options,
            Err(result) => return result,
        };

        let filter = match compile_filter(&options.filter) {
            Ok(filter) => filter,
            Err(err) => {
                return CommandResult::with_exit_code(
                    String::new(),
                    format!("yq: {}\n", err.message.trim_end()),
                    err.exit_code,
                );
            }
        };

        let sources = match load_sources(&ctx, &options).await {
            Ok(sources) => sources,
            Err(result) => return result,
        };

        if options.inplace {
            if sources.iter().any(|source| source.name == "stdin") {
                return CommandResult::with_exit_code(
                    String::new(),
                    "yq: --inplace requires file arguments\n".to_string(),
                    2,
                );
            }

            let mut last_output_truthy = false;
            for source in &sources {
                let output_format = resolve_output_format(&options, source.format);
                if let Err(result) = validate_output_mode(options.raw, output_format) {
                    return result;
                }

                let inputs = if options.null_input {
                    Vec::new()
                } else {
                    match collect_source_inputs(source, options.slurp) {
                        Ok(inputs) => inputs,
                        Err(err) => {
                            return CommandResult::with_exit_code(
                                String::new(),
                                format!("yq: parse error: {err}\n"),
                                5,
                            );
                        }
                    }
                };

                let output = {
                    let values = match run_filter(&filter, options.null_input, inputs) {
                        Ok(values) => values,
                        Err(err) => {
                            return CommandResult::with_exit_code(
                                String::new(),
                                format!("yq: error: {err}\n"),
                                5,
                            );
                        }
                    };
                    last_output_truthy = !values.is_empty() && last_truthy(&values) == Some(true);

                    let format = if options.raw {
                        Format::Raw
                    } else {
                        output_format
                    };
                    match format_values(
                        &values,
                        format,
                        options.compact,
                        options.join_output,
                        options.sort_keys,
                        options.indent,
                        options.use_tab,
                    ) {
                        Ok(output) => output,
                        Err(err) => {
                            return CommandResult::with_exit_code(
                                String::new(),
                                format!("yq: error: {err}\n"),
                                5,
                            );
                        }
                    }
                };

                let path = ctx.fs.resolve_path(&ctx.cwd, &source.name);
                if let Err(_) = ctx.fs.write_file(&path, output.as_bytes()).await {
                    return CommandResult::with_exit_code(
                        String::new(),
                        format!("yq: {}: write error\n", source.name),
                        2,
                    );
                }
            }

            let exit_code = if options.exit_status && !last_output_truthy {
                1
            } else {
                0
            };
            return CommandResult::with_exit_code(String::new(), String::new(), exit_code);
        }

        let output_format = resolve_output_format(&options, Format::Yaml);
        if let Err(result) = validate_output_mode(options.raw, output_format) {
            return result;
        }

        let inputs = if options.null_input {
            Vec::new()
        } else {
            let mut all_inputs = Vec::new();
            for source in &sources {
                match collect_source_inputs(source, false) {
                    Ok(mut values) => all_inputs.append(&mut values),
                    Err(err) => {
                        return CommandResult::with_exit_code(
                            String::new(),
                            format!("yq: parse error: {err}\n"),
                            5,
                        );
                    }
                }
            }
            if options.slurp {
                vec![all_inputs.into_iter().collect()]
            } else {
                all_inputs
            }
        };

        let values = match run_filter(&filter, options.null_input, inputs) {
            Ok(values) => values,
            Err(err) => {
                return CommandResult::with_exit_code(
                    String::new(),
                    format!("yq: error: {err}\n"),
                    5,
                );
            }
        };

        let format = if options.raw {
            Format::Raw
        } else {
            output_format
        };
        let stdout = match format_values(
            &values,
            format,
            options.compact,
            options.join_output,
            options.sort_keys,
            options.indent,
            options.use_tab,
        ) {
            Ok(stdout) => stdout,
            Err(err) => {
                return CommandResult::with_exit_code(
                    String::new(),
                    format!("yq: error: {err}\n"),
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
    use crate::fs::{FileSystem, InMemoryFs};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_ctx(args: &[&str], stdin: &str) -> CommandContext {
        CommandContext {
            args: args.iter().map(|arg| arg.to_string()).collect(),
            stdin: stdin.to_string(),
            cwd: "/".to_string(),
            env: HashMap::new(),
            fs: Arc::new(InMemoryFs::new()),
            exec_fn: None,
            fetch_fn: None,
        }
    }

    async fn make_ctx_with_files(
        args: &[&str],
        stdin: &str,
        files: &[(&str, &str)],
    ) -> CommandContext {
        let fs = Arc::new(InMemoryFs::new());
        for (path, content) in files {
            fs.write_file(path, content.as_bytes()).await.unwrap();
        }

        CommandContext {
            args: args.iter().map(|arg| arg.to_string()).collect(),
            stdin: stdin.to_string(),
            cwd: "/".to_string(),
            env: HashMap::new(),
            fs,
            exec_fn: None,
            fetch_fn: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_defaults_to_yaml_input_and_jq_output() {
        let result = YqCommand
            .execute(make_ctx(&[".name"], "name: hello\n"))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "\"hello\"\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_raw_output() {
        let result = YqCommand
            .execute(make_ctx(&["-r", ".name"], "name: hello\n"))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_yaml_output() {
        let result = YqCommand
            .execute(make_ctx(&["-y", ".metadata"], "metadata:\n  name: demo\n"))
            .await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("name: demo"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_toml_output() {
        let result = YqCommand
            .execute(make_ctx(&["-t", "."], "app:\n  name: demo\n  version: 2\n"))
            .await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("[app]"));
        assert!(result.stdout.contains("name = \"demo\""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_json_input_flag() {
        let result = YqCommand
            .execute(make_ctx(&["-J", ".name", "-r"], r#"{"name":"demo"}"#))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "demo\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_toml_input_flag() {
        let result = YqCommand
            .execute(make_ctx(
                &["-T", ".package.name", "-r"],
                "[package]\nname = \"demo\"\n",
            ))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "demo\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_slurp_yaml_documents() {
        let result = YqCommand
            .execute(make_ctx(
                &["-s", "length"],
                "---\nname: first\n---\nname: second\n",
            ))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "2\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_null_input() {
        let result = YqCommand
            .execute(make_ctx(&["-nc", r#"{"created": true}"#], ""))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "{\"created\":true}\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_compact_and_join_output() {
        let result = YqCommand
            .execute(make_ctx(
                &["-cj", ".items[]"],
                "items:\n  - 1\n  - 2\n  - 3\n",
            ))
            .await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "123");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_inplace_preserves_yaml_format() {
        let ctx = make_ctx_with_files(
            &["-i", r#".version = "2.0""#, "/data.yaml"],
            "",
            &[("/data.yaml", "version: 1.0\nname: test\n")],
        )
        .await;
        let fs = ctx.fs.clone();
        let result = YqCommand.execute(ctx).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "");
        let updated = fs.read_file("/data.yaml").await.unwrap();
        assert!(updated.contains("version: 2.0") || updated.contains("version: \"2.0\""));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_auto_detects_json_extension() {
        let ctx = make_ctx_with_files(
            &[".name", "/data.json"],
            "",
            &[("/data.json", r#"{"name":"demo"}"#)],
        )
        .await;
        let result = YqCommand.execute(ctx).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "\"demo\"\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_auto_detects_toml_extension() {
        let ctx = make_ctx_with_files(
            &["-r", ".package.name", "/Cargo.toml"],
            "",
            &[("/Cargo.toml", "[package]\nname = \"demo\"\n")],
        )
        .await;
        let result = YqCommand.execute(ctx).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "demo\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_invalid_yaml() {
        let result = YqCommand
            .execute(make_ctx(&["."], "invalid: yaml: syntax: error:"))
            .await;
        assert_eq!(result.exit_code, 5);
        assert!(result.stderr.contains("parse error"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_invalid_format() {
        let result = YqCommand.execute(make_ctx(&["-p", "xml", "."], "")).await;
        assert_eq!(result.exit_code, 2);
        assert!(result.stderr.contains("Unknown format"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_yq_raw_output_only_for_json_mode() {
        let result = YqCommand
            .execute(make_ctx(&["-yr", "."], "name: test\n"))
            .await;
        assert_eq!(result.exit_code, 2);
        assert!(result.stderr.contains("raw-output"));
    }
}
