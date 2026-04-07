// src/commands/printf/mod.rs
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;
use sprintf::Printf;

pub struct PrintfCommand;

const HELP: &str = "Usage: printf FORMAT [ARGUMENT]...\n\n\
Print ARGUMENT(s) according to FORMAT.\n\n\
FORMAT is a string with escape sequences and format specifiers:\n  %%  literal %\n  %s  string\n  %d  decimal integer\n  %f  floating point\n  %x  hexadecimal\n  %o  octal\n  %c  character\n  %b  string with escape interpretation\n\n\
Escape sequences:\n  \\\\  backslash  \\n  newline  \\t  tab  \\r  carriage return\n  \\xHH hex  \\0NNN octal  \\uHHHH unicode  \\UHHHHHHHH unicode\n";

#[async_trait]
impl Command for PrintfCommand {
    fn name(&self) -> &'static str {
        "printf"
    }

    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        let args = &ctx.args;
        if args.is_empty() {
            return CommandResult::with_exit_code(
                "".into(),
                "printf: usage: printf format [arguments]\n".into(),
                2,
            );
        }
        if args.iter().any(|a| a == "--help") {
            return CommandResult::success(HELP.into());
        }

        let format = &args[0];
        let arguments: Vec<&str> = args[1..].iter().map(|s| s.as_str()).collect();
        let mut output = String::new();
        let mut stderr = String::new();
        let mut exit_code = 0;
        let mut arg_idx = 0;

        // Process format string, repeating if we have unused arguments
        loop {
            let start_arg_idx = arg_idx;
            let chars: Vec<char> = format.chars().collect();
            let mut i = 0;

            while i < chars.len() {
                if chars[i] == '\\' {
                    let (esc, advance) = process_escape(&chars, i);
                    output.push_str(&esc);
                    i += advance;
                } else if chars[i] == '%' {
                    if i + 1 >= chars.len() {
                        output.push('%');
                        i += 1;
                        continue;
                    }
                    if chars[i + 1] == '%' {
                        output.push('%');
                        i += 2;
                        continue;
                    }
                    let (formatted, advance, consumed, parse_err) =
                        process_format_spec(&chars, i, &arguments, arg_idx);
                    output.push_str(&formatted);
                    arg_idx += consumed;
                    i += advance;
                    if let Some(err_msg) = parse_err {
                        stderr.push_str(&err_msg);
                        exit_code = 1;
                    }
                } else {
                    output.push(chars[i]);
                    i += 1;
                }
            }

            if arg_idx <= start_arg_idx || arg_idx >= arguments.len() {
                break;
            }
        }

        CommandResult::with_exit_code(output, stderr, exit_code)
    }
}

fn process_escape(chars: &[char], pos: usize) -> (String, usize) {
    if pos + 1 >= chars.len() {
        return ("\\".into(), 1);
    }
    let next = chars[pos + 1];
    match next {
        '\\' => ("\\".into(), 2),
        'n' => ("\n".into(), 2),
        't' => ("\t".into(), 2),
        'r' => ("\r".into(), 2),
        'a' => ("\x07".into(), 2),
        'b' => ("\x08".into(), 2),
        'f' => ("\x0c".into(), 2),
        'v' => ("\x0b".into(), 2),
        'e' | 'E' => ("\x1b".into(), 2),
        '0' => parse_octal_escape(chars, pos + 2, pos),
        'x' => parse_hex_escape(chars, pos + 2, 2, pos),
        'u' => parse_hex_escape(chars, pos + 2, 4, pos),
        'U' => parse_hex_escape(chars, pos + 2, 8, pos),
        '1'..='7' => {
            let mut oct = String::new();
            oct.push(next);
            let mut j = pos + 2;
            while j < chars.len() && j < pos + 4 && chars[j] >= '0' && chars[j] <= '7' {
                oct.push(chars[j]);
                j += 1;
            }
            let code = u32::from_str_radix(&oct, 8).unwrap_or(0) % 256;
            (char::from_u32(code).map_or(String::new(), |c| c.to_string()), j - pos)
        }
        _ => {
            let mut s = String::from('\\');
            s.push(next);
            (s, 2)
        }
    }
}

fn parse_octal_escape(chars: &[char], start: usize, base_pos: usize) -> (String, usize) {
    let mut oct = String::new();
    let mut j = start;
    while j < chars.len() && j < start + 3 && chars[j] >= '0' && chars[j] <= '7' {
        oct.push(chars[j]);
        j += 1;
    }
    let code = if oct.is_empty() { 0 } else { u32::from_str_radix(&oct, 8).unwrap_or(0) % 256 };
    (char::from_u32(code).map_or(String::new(), |c| c.to_string()), j - base_pos)
}

fn parse_hex_escape(chars: &[char], start: usize, max_digits: usize, base_pos: usize) -> (String, usize) {
    let mut hex = String::new();
    let mut j = start;
    while j < chars.len() && j < start + max_digits && chars[j].is_ascii_hexdigit() {
        hex.push(chars[j]);
        j += 1;
    }
    if hex.is_empty() {
        let prefix: String = chars[base_pos..start].iter().collect();
        (prefix, start - base_pos)
    } else {
        let code = u32::from_str_radix(&hex, 16).unwrap_or(0);
        (char::from_u32(code).map_or(String::new(), |c| c.to_string()), j - base_pos)
    }
}

/// Returns (formatted_str, chars_advanced, args_consumed, optional_error_msg)
fn process_format_spec(
    chars: &[char],
    pos: usize,
    args: &[&str],
    arg_idx: usize,
) -> (String, usize, usize, Option<String>) {
    let mut i = pos + 1;
    // Collect flags
    while i < chars.len() && "-+ 0#'".contains(chars[i]) {
        i += 1;
    }
    // Collect width
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    // Collect precision
    if i < chars.len() && chars[i] == '.' {
        i += 1;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i >= chars.len() {
        return ("%".into(), 1, 0, None);
    }

    let specifier = chars[i];
    let spec_str: String = chars[pos..=i].iter().collect();
    let advance = i - pos + 1;
    let arg = if arg_idx < args.len() { args[arg_idx] } else { "" };

    match specifier {
        's' => {
            let formatted = sprintf::vsprintf(&spec_str, &[&arg as &dyn Printf]).unwrap_or_else(|_| arg.to_string());
            (formatted, advance, 1, None)
        }
        'd' | 'i' => {
            let (val, err) = parse_int_arg(arg);
            let formatted = sprintf::vsprintf(&spec_str, &[&val as &dyn Printf]).unwrap_or_else(|_| format!("{}", val));
            (formatted, advance, 1, err)
        }
        'f' | 'g' | 'e' | 'G' | 'E' => {
            let (val, err) = parse_float_arg(arg);
            let formatted = sprintf::vsprintf(&spec_str, &[&val as &dyn Printf]).unwrap_or_else(|_| format!("{}", val));
            (formatted, advance, 1, err)
        }
        'x' | 'X' => {
            let (val, err) = parse_int_arg(arg);
            let uval = val as u64;
            let formatted = sprintf::vsprintf(&spec_str, &[&uval as &dyn Printf]).unwrap_or_else(|_| format!("{:x}", uval));
            (formatted, advance, 1, err)
        }
        'o' => {
            let (val, err) = parse_int_arg(arg);
            let uval = val as u64;
            let formatted = sprintf::vsprintf(&spec_str, &[&uval as &dyn Printf]).unwrap_or_else(|_| format!("{:o}", uval));
            (formatted, advance, 1, err)
        }
        'c' => {
            let ch = arg.chars().next().unwrap_or('\0');
            if ch == '\0' {
                (String::new(), advance, 1, None)
            } else {
                (ch.to_string(), advance, 1, None)
            }
        }
        'b' => {
            let processed = process_b_escape(arg);
            (processed, advance, 1, None)
        }
        'q' => {
            let quoted = shell_quote(arg);
            (quoted, advance, 1, None)
        }
        _ => (spec_str, advance, 0, None),
    }
}

fn parse_int_arg(s: &str) -> (i64, Option<String>) {
    if s.is_empty() {
        return (0, None);
    }
    // Handle quoted character
    if s.len() >= 2 && (s.starts_with('\'') || s.starts_with('"')) {
        let ch = s.chars().nth(1).unwrap_or('\0');
        return (ch as i64, None);
    }
    // Handle hex
    if s.starts_with("0x") || s.starts_with("0X") {
        return match i64::from_str_radix(&s[2..], 16) {
            Ok(v) => (v, None),
            Err(_) => (0, Some(format!("printf: '{}': invalid number\n", s))),
        };
    }
    // Handle octal
    if s.starts_with('0') && s.len() > 1 && s.chars().skip(1).all(|c| c >= '0' && c <= '7') {
        return match i64::from_str_radix(&s[1..], 8) {
            Ok(v) => (v, None),
            Err(_) => (0, Some(format!("printf: '{}': invalid number\n", s))),
        };
    }
    match s.parse::<i64>() {
        Ok(v) => (v, None),
        Err(_) => (0, Some(format!("printf: '{}': invalid number\n", s))),
    }
}

fn parse_float_arg(s: &str) -> (f64, Option<String>) {
    if s.is_empty() {
        return (0.0, None);
    }
    match s.parse::<f64>() {
        Ok(v) => (v, None),
        Err(_) => (0.0, Some(format!("printf: '{}': invalid number\n", s))),
    }
}

fn process_b_escape(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            let (esc, adv) = process_escape(&chars, i);
            result.push_str(&esc);
            i += adv;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars().all(|c| c.is_alphanumeric() || "-_./,:@".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_utils::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_string() {
        let r = PrintfCommand.execute(make_ctx(vec!["Hello %s", "world"])).await;
        assert_eq!(r.stdout, "Hello world");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_int() {
        let r = PrintfCommand.execute(make_ctx(vec!["Number: %d", "42"])).await;
        assert_eq!(r.stdout, "Number: 42");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_float() {
        let r = PrintfCommand.execute(make_ctx(vec!["Value: %f", "3.14"])).await;
        assert_eq!(r.stdout, "Value: 3.140000");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_hex() {
        let r = PrintfCommand.execute(make_ctx(vec!["Hex: %x", "255"])).await;
        assert_eq!(r.stdout, "Hex: ff");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_octal() {
        let r = PrintfCommand.execute(make_ctx(vec!["Octal: %o", "8"])).await;
        assert_eq!(r.stdout, "Octal: 10");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_percent() {
        let r = PrintfCommand.execute(make_ctx(vec!["100%%"])).await;
        assert_eq!(r.stdout, "100%");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_multi() {
        let r = PrintfCommand.execute(make_ctx(vec!["%s is %d years old", "Alice", "30"])).await;
        assert_eq!(r.stdout, "Alice is 30 years old");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_newline() {
        let r = PrintfCommand.execute(make_ctx(vec!["line1\\nline2"])).await;
        assert_eq!(r.stdout, "line1\nline2");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_tab() {
        let r = PrintfCommand.execute(make_ctx(vec!["col1\\tcol2"])).await;
        assert_eq!(r.stdout, "col1\tcol2");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_width() {
        let r = PrintfCommand.execute(make_ctx(vec!["%10s", "hi"])).await;
        assert_eq!(r.stdout, "        hi");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_prec() {
        let r = PrintfCommand.execute(make_ctx(vec!["%.2f", "3.14159"])).await;
        assert_eq!(r.stdout, "3.14");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_zero_pad() {
        let r = PrintfCommand.execute(make_ctx(vec!["%05d", "42"])).await;
        assert_eq!(r.stdout, "00042");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_left() {
        let r = PrintfCommand.execute(make_ctx(vec!["%-10s|", "hi"])).await;
        assert_eq!(r.stdout, "hi        |");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_no_args() {
        let r = PrintfCommand.execute(make_ctx(vec![])).await;
        assert_eq!(r.exit_code, 2);
        assert!(r.stderr.contains("usage"));
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_missing_args() {
        let r = PrintfCommand.execute(make_ctx(vec!["%s %s", "only"])).await;
        assert_eq!(r.stdout, "only ");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_invalid_num() {
        let r = PrintfCommand.execute(make_ctx(vec!["%d", "notanumber"])).await;
        assert_eq!(r.stdout, "0");
        assert_eq!(r.exit_code, 1);
        assert!(r.stderr.contains("invalid number"));
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_help() {
        let r = PrintfCommand.execute(make_ctx(vec!["--help"])).await;
        assert!(r.stdout.contains("printf"));
        assert!(r.stdout.contains("FORMAT"));
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_escape_e() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\e[31mred\\e[0m"])).await;
        assert_eq!(r.stdout, "\x1b[31mred\x1b[0m");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_unicode() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\u2764"])).await;
        assert_eq!(r.stdout, "❤");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_unicode_u() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\U1F600"])).await;
        assert_eq!(r.stdout, "😀");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_hex_x() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\x41\\x42\\x43"])).await;
        assert_eq!(r.stdout, "ABC");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_octal_esc() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\101\\102\\103"])).await;
        assert_eq!(r.stdout, "ABC");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_backslash() {
        let r = PrintfCommand.execute(make_ctx(vec!["x\\\\y"])).await;
        assert_eq!(r.stdout, "x\\y");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_carriage_return() {
        let r = PrintfCommand.execute(make_ctx(vec!["hello\\rworld"])).await;
        assert_eq!(r.stdout, "hello\rworld");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_escape_capital_e() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\E[1mbold\\E[0m"])).await;
        assert_eq!(r.stdout, "\x1b[1mbold\x1b[0m");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_bell() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\a"])).await;
        assert_eq!(r.stdout, "\x07");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_backspace() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\b"])).await;
        assert_eq!(r.stdout, "\x08");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_form_feed() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\f"])).await;
        assert_eq!(r.stdout, "\x0c");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_vertical_tab() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\v"])).await;
        assert_eq!(r.stdout, "\x0b");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_binary_hex_escapes() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\x41\\x42\\x43"])).await;
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, "ABC");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_null_bytes_hex() {
        let r = PrintfCommand.execute(make_ctx(vec!["A\\x00B\\x00C"])).await;
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, "A\0B\0C");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_lowercase_hex() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\x61\\x62\\x63"])).await;
        assert_eq!(r.stdout, "abc");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_mixed_case_hex() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\xAa"])).await;
        assert_eq!(r.stdout, "\u{aa}");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_binary_octal_escapes() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\101\\102\\103"])).await;
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, "ABC");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_octal_null() {
        let r = PrintfCommand.execute(make_ctx(vec!["a\\0b"])).await;
        assert_eq!(r.stdout, "a\0b");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_octal_max_three_digits() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\0101"])).await;
        assert_eq!(r.stdout, "A");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_octal_question_mark() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\077"])).await;
        assert_eq!(r.stdout, "?");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_unicode_fewer_digits() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\u41"])).await;
        assert_eq!(r.stdout, "A");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_unicode_checkmark() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\u2714"])).await;
        assert_eq!(r.stdout, "✔");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_unicode_capital_u_fewer_digits() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\U1F4C4"])).await;
        assert_eq!(r.stdout, "📄");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_unicode_rocket() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\U1F680"])).await;
        assert_eq!(r.stdout, "🚀");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_combined_ansi_unicode() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\e[31m\\u2764\\e[0m\\n"])).await;
        assert_eq!(r.stdout, "\x1b[31m❤\x1b[0m\n");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_complex_unicode_with_tabs() {
        let r = PrintfCommand.execute(make_ctx(vec!["\\U1F4C1 folder\\t\\U1F4C4 file"])).await;
        assert_eq!(r.stdout, "📁 folder\t📄 file");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_hex_uppercase() {
        let r = PrintfCommand.execute(make_ctx(vec!["Hex: %X", "255"])).await;
        assert_eq!(r.stdout, "Hex: FF");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_width_and_precision() {
        let r = PrintfCommand.execute(make_ctx(vec!["%-10.3s", "hello"])).await;
        assert_eq!(r.stdout, "hel       ");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_precision_truncate() {
        let r = PrintfCommand.execute(make_ctx(vec!["%.3s", "hello"])).await;
        assert_eq!(r.stdout, "hel");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_char_format() {
        let r = PrintfCommand.execute(make_ctx(vec!["%c", "A"])).await;
        assert_eq!(r.stdout, "A");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_char_format_empty() {
        let r = PrintfCommand.execute(make_ctx(vec!["%c", ""])).await;
        assert_eq!(r.stdout, "");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_format_reuse() {
        let r = PrintfCommand.execute(make_ctx(vec!["%s\\n", "first", "second", "third"])).await;
        assert_eq!(r.stdout, "first\nsecond\nthird\n");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_plus_flag() {
        let r = PrintfCommand.execute(make_ctx(vec!["%+d", "42"])).await;
        assert_eq!(r.stdout, "+42");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_space_flag() {
        let r = PrintfCommand.execute(make_ctx(vec!["% d", "42"])).await;
        assert_eq!(r.stdout, " 42");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_hex_input() {
        let r = PrintfCommand.execute(make_ctx(vec!["%d", "0xff"])).await;
        assert_eq!(r.stdout, "255");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_octal_input() {
        let r = PrintfCommand.execute(make_ctx(vec!["%d", "010"])).await;
        assert_eq!(r.stdout, "8");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_quoted_char_single() {
        let r = PrintfCommand.execute(make_ctx(vec!["%d", "'A"])).await;
        assert_eq!(r.stdout, "65");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_quoted_char_double() {
        let r = PrintfCommand.execute(make_ctx(vec!["%d", "\"B"])).await;
        assert_eq!(r.stdout, "66");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_b_format() {
        let r = PrintfCommand.execute(make_ctx(vec!["%b", "hello\\nworld"])).await;
        assert_eq!(r.stdout, "hello\nworld");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_b_format_tab() {
        let r = PrintfCommand.execute(make_ctx(vec!["%b", "col1\\tcol2"])).await;
        assert_eq!(r.stdout, "col1\tcol2");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_q_format_simple() {
        let r = PrintfCommand.execute(make_ctx(vec!["%q", "hello"])).await;
        assert_eq!(r.stdout, "hello");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_q_format_with_spaces() {
        let r = PrintfCommand.execute(make_ctx(vec!["%q", "hello world"])).await;
        assert_eq!(r.stdout, "'hello world'");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_q_format_empty() {
        let r = PrintfCommand.execute(make_ctx(vec!["%q", ""])).await;
        assert_eq!(r.stdout, "''");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_exponential() {
        let r = PrintfCommand.execute(make_ctx(vec!["%.2e", "1234.5"])).await;
        assert_eq!(r.exit_code, 0);
        assert!(!r.stdout.is_empty());
        assert!(r.stdout.contains('e'));
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_printf_general_format() {
        let r = PrintfCommand.execute(make_ctx(vec!["%.2g", "1234.5"])).await;
        assert_eq!(r.exit_code, 0);
        assert!(!r.stdout.is_empty());
    }
}
