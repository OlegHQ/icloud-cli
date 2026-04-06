use crate::commands::errors::no_such_file;
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;
use regex_lite::Regex;

pub struct HtmlToMarkdownCommand;

#[async_trait]
impl Command for HtmlToMarkdownCommand {
    fn name(&self) -> &'static str {
        "html-to-markdown"
    }

    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        if ctx.args.iter().any(|a| a == "--help") {
            return CommandResult::success(
                "html-to-markdown - convert HTML to Markdown\n\nUsage: html-to-markdown [FILE]\n\nConvert HTML to Markdown format.\n".to_string()
            );
        }

        let input = if ctx.args.is_empty() || ctx.args[0] == "-" {
            ctx.stdin.clone()
        } else {
            let path = ctx.fs.resolve_path(&ctx.cwd, &ctx.args[0]);
            match ctx.fs.read_file(&path).await {
                Ok(c) => c,
                Err(_) => {
                    return CommandResult::error(no_such_file("html-to-markdown", &ctx.args[0]))
                }
            }
        };

        if input.trim().is_empty() {
            return CommandResult::success(String::new());
        }

        let markdown = html_to_markdown(&input);
        CommandResult::success(format!("{}\n", markdown.trim()))
    }
}

fn html_to_markdown(html: &str) -> String {
    // Strip <script> and <style> blocks before conversion (htmd does not remove them).
    let script_re = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let cleaned = script_re.replace_all(html, "");
    let style_re = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let cleaned = style_re.replace_all(&cleaned, "");

    htmd::convert(&cleaned).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::InMemoryFs;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn create_ctx(args: Vec<&str>) -> CommandContext {
        CommandContext {
            args: args.into_iter().map(String::from).collect(),
            stdin: String::new(),
            cwd: "/".to_string(),
            env: HashMap::new(),
            fs: Arc::new(InMemoryFs::new()),
            exec_fn: None,
            fetch_fn: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_help() {
        let ctx = create_ctx(vec!["--help"]);
        let result = HtmlToMarkdownCommand.execute(ctx).await;
        assert!(result.stdout.contains("html-to-markdown"));
        assert!(result.stdout.contains("Markdown"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_empty_input() {
        let ctx = create_ctx(vec![]);
        let result = HtmlToMarkdownCommand.execute(ctx).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_headings() {
        let mut ctx = create_ctx(vec![]);
        ctx.stdin = "<h1>Title</h1><h2>Subtitle</h2>".to_string();
        let result = HtmlToMarkdownCommand.execute(ctx).await;
        assert!(result.stdout.contains("# Title"));
        assert!(result.stdout.contains("## Subtitle"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_bold_italic() {
        let mut ctx = create_ctx(vec![]);
        ctx.stdin = "<strong>bold</strong> and <em>italic</em>".to_string();
        let result = HtmlToMarkdownCommand.execute(ctx).await;
        assert!(result.stdout.contains("**bold**"));
        assert!(result.stdout.contains("*italic*"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_links() {
        let mut ctx = create_ctx(vec![]);
        ctx.stdin = r#"<a href="https://example.com">link</a>"#.to_string();
        let result = HtmlToMarkdownCommand.execute(ctx).await;
        assert!(result.stdout.contains("[link](https://example.com)"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_code() {
        let mut ctx = create_ctx(vec![]);
        ctx.stdin = "<code>inline</code> and <pre><code>block</code></pre>".to_string();
        let result = HtmlToMarkdownCommand.execute(ctx).await;
        assert!(result.stdout.contains("`inline`"));
        assert!(result.stdout.contains("```"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_list() {
        let mut ctx = create_ctx(vec![]);
        ctx.stdin = "<ul><li>one</li><li>two</li></ul>".to_string();
        let result = HtmlToMarkdownCommand.execute(ctx).await;
        // htmd uses * for unordered list items
        assert!(result.stdout.contains("one"));
        assert!(result.stdout.contains("two"));
        // Verify list markers are present (htmd uses *)
        assert!(result.stdout.contains("*"));

    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_strip_script_style() {
        let mut ctx = create_ctx(vec![]);
        ctx.stdin = "<script>alert(1)</script><style>.x{}</style><p>text</p>".to_string();
        let result = HtmlToMarkdownCommand.execute(ctx).await;
        assert!(!result.stdout.contains("alert"));
        assert!(!result.stdout.contains(".x"));
        assert!(result.stdout.contains("text"));
    }

    #[test]
    fn test_html_to_markdown_fn() {
        assert_eq!(html_to_markdown("<p>hello</p>").trim(), "hello");
        assert_eq!(html_to_markdown("<h1>title</h1>").trim(), "# title");
    }

    #[test]
    fn test_html_to_markdown_strips_tags() {
        // Verifies that raw tags are not present in output
        let result = html_to_markdown("<p>hello</p>");
        assert!(result.contains("hello"));
        assert!(!result.contains("<p>"));
    }

}
