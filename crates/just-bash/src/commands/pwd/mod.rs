// src/commands/pwd/mod.rs
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;

pub struct PwdCommand;

#[async_trait]
impl Command for PwdCommand {
    fn name(&self) -> &'static str {
        "pwd"
    }

    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        let args = &ctx.args;

        // Parse options
        let mut use_physical = false;

        for arg in args {
            match arg.as_str() {
                "-P" => use_physical = true,
                "-L" => use_physical = false,
                "--" => break,
                _ if arg.starts_with('-') => {
                    // Ignore unknown options (bash behavior)
                }
                _ => {}
            }
        }

        let mut pwd = ctx.cwd.clone();

        if use_physical {
            // -P: resolve all symlinks to get physical path
            if let Ok(real) = ctx.fs.realpath(&ctx.cwd).await {
                pwd = real;
            }
            // If realpath fails, fall back to current cwd (bash behavior)
        }

        CommandResult::success(format!("{}\n", pwd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_utils::*;

    fn make_ctx(args: Vec<&str>, cwd: &str) -> CommandContext {
        let mut ctx = crate::commands::test_utils::make_ctx(args);
        ctx.cwd = cwd.to_string();
        ctx
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_pwd_default() {
        let ctx = make_ctx(vec![], "/home/user");
        let cmd = PwdCommand;
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "/home/user\n");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_pwd_root() {
        let ctx = make_ctx(vec![], "/");
        let cmd = PwdCommand;
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "/\n");
        assert_eq!(result.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_pwd_ignore_args() {
        let ctx = make_ctx(vec!["ignored", "args"], "/test");
        let cmd = PwdCommand;
        let result = cmd.execute(ctx).await;
        assert_eq!(result.stdout, "/test\n");
    }
}
