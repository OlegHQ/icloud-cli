use super::*;
use crate::commands::test_utils::*;
use crate::fs::FileSystem;

fn make_ctx(args: Vec<&str>, stdin: &str) -> CommandContext {
    make_ctx_with_stdin(args, stdin)
}

fn make_ctx_with_fs(args: Vec<&str>, stdin: &str, fs: Arc<InMemoryFs>) -> CommandContext {
    make_ctx_with_stdin_and_fs(args, stdin, fs)
}

fn make_ctx_with_env(args: Vec<&str>, stdin: &str, env: HashMap<String, String>) -> CommandContext {
    make_ctx_with_stdin_and_env(args, stdin, env)
}

// ─── Basic Functionality Tests ────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_print_all_lines() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print }"], "line1\nline2\nline3\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "line1\nline2\nline3\n");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_print_first_field() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print $1 }"], "hello world\nfoo bar\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "hello\nfoo\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_print_multiple_fields_with_ofs() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print $1, $2 }"], "a b c\nx y z\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a b\nx y\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_custom_field_separator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["-F:", "{ print $1 }"],
        "root:x:0:0\nuser:x:1000:1000\n",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "root\nuser\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_custom_field_separator_combined() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["-F:", "{ print $2 }"], "a:b:c\nx:y:z\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "b\ny\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_preset_variable() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["-v", "x=10", "{ print x }"], "line\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "10\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_begin_main_end() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { print \"start\" } { print } END { print \"end\" }"],
        "middle\n",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "start\nmiddle\nend\n");
}

// ─── Pattern Tests ────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_regex_pattern() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["/foo/ { print }"], "foo\nbar\nfoobar\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "foo\nfoobar\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_expression_pattern() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["NR > 2 { print }"], "a\nb\nc\nd\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "c\nd\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_range_pattern() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["/start/,/end/ { print }"],
        "before\nstart\nmiddle\nend\nafter\n",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "start\nmiddle\nend\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pattern_without_action() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["NR == 2"], "a\nb\nc\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "b\n");
}

// ─── Operator Tests ───────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_arithmetic_operators() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print $1 + $2 }"], "3 4\n10 5\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "7\n15\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_string_concatenation() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print $1 $2 }"], "hello world\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "helloworld\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_comparison_operators() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["$1 > 10 { print }"], "5\n15\n8\n20\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "15\n20\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_regex_match_operator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["$1 ~ /^a/ { print }"], "apple\nbanana\napricot\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "apple\napricot\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ternary_operator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print ($1 > 0 ? \"pos\" : \"neg\") }"], "5\n-3\n0\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "pos\nneg\nneg\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_assignment_accumulator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ sum += $1 } END { print sum }"], "1\n2\n3\n4\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "10\n");
}

// ─── Control Flow Tests ───────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_if_else() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["{ if ($1 > 5) print \"big\"; else print \"small\" }"],
        "3\n8\n5\n",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "small\nbig\nsmall\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_while_loop() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { i=1; while (i <= 3) { print i; i++ } }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1\n2\n3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_for_loop() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { for (i=1; i<=3; i++) print i }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1\n2\n3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_for_in_loop() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { a[1]=10; a[2]=20; for (k in a) sum += a[k]; print sum }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "30\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_break_statement() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { for (i=1; i<=10; i++) { if (i > 3) break; print i } }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1\n2\n3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_continue_statement() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { for (i=1; i<=5; i++) { if (i == 3) continue; print i } }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1\n2\n4\n5\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_next_statement() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["/skip/ { next } { print }"], "a\nskip\nb\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a\nb\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_exit_statement() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print; if (NR == 2) exit }"], "a\nb\nc\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a\nb\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_exit_with_code() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { exit 42 }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.exit_code, 42);
}

// ─── Function Tests ───────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_user_defined_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["function double(x) { return x * 2 } BEGIN { print double(5) }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "10\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_length_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print length($0) }"], "hello\nhi\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "5\n2\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_substr_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print substr($0, 2, 3) }"], "hello\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "ell\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_index_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print index($0, \"ll\") }"], "hello\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_split_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["{ n = split($0, arr, \":\"); print n, arr[1], arr[2] }"],
        "a:b:c\n",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "3 a b\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ sub(/o/, \"0\"); print }"], "foo\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "f0o\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gsub_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ gsub(/o/, \"0\"); print }"], "foo\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "f00\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_match_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print match($0, \"abc\") }"], "xyzabcdef\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "4\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_tolower_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print tolower($0) }"], "HELLO\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "hello\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_toupper_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print toupper($0) }"], "hello\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "HELLO\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_int_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { print int(3.7) }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sqrt_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { print sqrt(16) }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "4\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sprintf_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { print sprintf(\"%d\", 42) }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "42\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_printf_statement() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { printf \"%s=%d\\n\", \"x\", 10 }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "x=10\n");
}

// ─── Array Tests ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_associative_array() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { a[\"x\"] = 1; a[\"y\"] = 2; print a[\"x\"], a[\"y\"] }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1 2\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_array_element() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { a[1]=10; a[2]=20; delete a[1]; for (k in a) print k, a[k] }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "2 20\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_delete_entire_array() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { a[1]=10; a[2]=20; delete a; for (k in a) print k }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_in_operator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { a[1]=10; if (1 in a) print \"yes\"; if (2 in a) print \"no\" }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "yes\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_dimensional_array() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { a[1,2] = 10; print a[1,2] }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "10\n");
}

// ─── Field Tests ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_field_zero_is_entire_line() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print $0 }"], "hello world\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "hello world\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_modify_field_rebuilds_line() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ $2 = \"X\"; print $0 }"], "a b c\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a X c\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_modify_line_resplits_fields() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ $0 = \"x y z\"; print $2 }"], "a b c\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "y\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_nf_is_last_field() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print $NF }"], "a b c\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "c\n");
}

// ─── Built-in Variable Tests ──────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_nr_variable() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print NR }"], "a\nb\nc\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1\n2\n3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_nf_variable() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print NF }"], "a b c\nx y\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "3\n2\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fs_ofs() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { FS=\":\"; OFS=\"-\" } { print $1, $2 }"],
        "a:b\nx:y\n",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a-b\nx-y\n");
}

// KNOWN_ISSUES.md: awk-rs 0.1 ignores custom ORS
#[ignore]
#[tokio::test(flavor = "multi_thread")]
async fn test_ors_known_issue() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { FS=\":\"; OFS=\"-\"; ORS=\"|\" } { print $1, $2 }"],
        "a:b\nx:y\n",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a-b|x-y|");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_argc_argv() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { print ARGC, ARGV[0], ARGV[1] }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1 awk \n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_environ_variable() {
    // awk-rs reads ENVIRON from the real process environment
    unsafe { std::env::set_var("AWK_TEST_VAR", "hello") };
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { print ENVIRON[\"AWK_TEST_VAR\"] }"], "");
    let result = cmd.execute(ctx).await;
    unsafe { std::env::remove_var("AWK_TEST_VAR") };
    assert_eq!(result.stdout, "hello\n");
}

// KNOWN_ISSUES.md: awk-rs reads ENVIRON from host process, not sandbox ctx.env
#[ignore]
#[tokio::test(flavor = "multi_thread")]
async fn test_environ_sandbox_known_issue() {
    let mut env = HashMap::new();
    env.insert("MY_VAR".to_string(), "hello".to_string());
    let cmd = AwkCommand;
    let ctx = make_ctx_with_env(vec!["BEGIN { print ENVIRON[\"MY_VAR\"] }"], "", env);
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "hello\n");
}

// ─── Edge Case Tests ──────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_empty_input() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "");
    assert_eq!(result.exit_code, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_files() {
    let fs = Arc::new(InMemoryFs::new());
    fs.write_file("/file1.txt", b"a\nb\n").await.unwrap();
    fs.write_file("/file2.txt", b"c\nd\n").await.unwrap();
    let cmd = AwkCommand;
    let ctx = CommandContext {
        args: vec![
            "{ print }".to_string(),
            "/file1.txt".to_string(),
            "/file2.txt".to_string(),
        ],
        stdin: String::new(),
        cwd: "/".to_string(),
        env: HashMap::new(),
        fs,
        exec_fn: None,
        fetch_fn: None,
    };
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a\nb\nc\nd\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_missing_program_error() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec![], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.exit_code, 1);
    assert!(result.stderr.contains("missing program"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_parse_error() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print "], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.exit_code, 1);
    assert!(!result.stderr.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_file_not_found() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print }", "nonexistent.txt"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.exit_code, 1);
    assert!(result.stderr.contains("No such file"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_help_flag() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["--help"], "");
    let result = cmd.execute(ctx).await;
    assert!(result.stdout.contains("Usage:"));
    assert!(result.stdout.contains("awk"));
    assert_eq!(result.exit_code, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_unknown_option() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["-x", "{ print }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.exit_code, 1);
    assert!(result.stderr.contains("unknown option"));
}

// ─── Additional Tests ─────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_begin_only_no_input() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { print \"hello\" }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "hello\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_end_only() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["END { print NR }"], "a\nb\nc\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_rules() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["/a/ { print \"has a\" } /b/ { print \"has b\" }"],
        "ab\na\nb\n",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "has a\nhas b\nhas a\nhas b\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_logical_and_pattern() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["NR > 1 && NR < 4 { print }"], "a\nb\nc\nd\ne\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "b\nc\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_logical_or_pattern() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["NR == 1 || NR == 3 { print }"], "a\nb\nc\nd\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a\nc\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_negation_pattern() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["!/skip/ { print }"], "a\nskip\nb\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a\nb\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_modulo_operator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ print $1 % 3 }"], "10\n7\n9\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1\n1\n0\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_exponentiation() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { print 2^10 }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1024\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_pre_increment() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { x=5; print ++x, x }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "6 6\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_post_increment() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { x=5; print x++, x }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "5 6\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_unary_minus() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { x=5; print -x }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "-5\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_not_operator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["BEGIN { print !0, !1, !\"\" }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1 0 1\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_string_comparison() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["$1 == \"foo\" { print \"match\" }"], "foo\nbar\nfoo\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "match\nmatch\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_not_match_operator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["$1 !~ /^a/ { print }"], "apple\nbanana\napricot\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "banana\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_do_while_loop() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { i=1; do { print i; i++ } while (i <= 3) }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1\n2\n3\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_nested_loops() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["BEGIN { for (i=1; i<=2; i++) for (j=1; j<=2; j++) print i, j }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "1 1\n1 2\n2 1\n2 2\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_recursive_function() {
    let cmd = AwkCommand;
    let ctx = make_ctx(
        vec!["function fact(n) { return n <= 1 ? 1 : n * fact(n-1) } BEGIN { print fact(5) }"],
        "",
    );
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "120\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gsub_return_value() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ n = gsub(/o/, \"0\"); print n, $0 }"], "foo\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "2 f00\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_sub_with_ampersand() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["{ sub(/o/, \"[&]\"); print }"], "foo\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "f[o]o\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_escape_in_field_separator() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["-F", "\\t", "{ print $1, $2 }"], "a\tb\tc\n");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a b\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_escape_in_variable() {
    let cmd = AwkCommand;
    let ctx = make_ctx(vec!["-v", "x=a\\tb", "BEGIN { print x }"], "");
    let result = cmd.execute(ctx).await;
    assert_eq!(result.stdout, "a\tb\n");
}
