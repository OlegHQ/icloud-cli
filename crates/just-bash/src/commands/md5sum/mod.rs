// src/commands/md5sum/mod.rs
// md5sum, sha1sum, sha256sum — checksum commands
use crate::commands::errors::no_such_file;
use crate::commands::{Command, CommandContext, CommandResult};
use async_trait::async_trait;
use digest::Digest;

#[derive(Clone, Copy)]
pub enum HashAlgorithm {
    Md5,
    Sha1,
    Sha256,
}

fn compute_hash(algorithm: HashAlgorithm, data: &[u8]) -> String {
    match algorithm {
        HashAlgorithm::Md5 => hex::encode(md5::Md5::digest(data)),
        HashAlgorithm::Sha1 => hex::encode(sha1::Sha1::digest(data)),
        HashAlgorithm::Sha256 => hex::encode(sha2::Sha256::digest(data)),
    }
}

fn algo_name(algo: HashAlgorithm) -> &'static str {
    match algo {
        HashAlgorithm::Md5 => "MD5",
        HashAlgorithm::Sha1 => "SHA1",
        HashAlgorithm::Sha256 => "SHA256",
    }
}

fn make_help(name: &str, summary: &str) -> String {
    format!("Usage: {} [OPTION]... [FILE]...\n\n{}\n\nOptions:\n  -c, --check    read checksums from FILEs and check them\n      --help     display this help and exit\n", name, summary)
}

async fn checksum_execute(
    name: &str,
    algorithm: HashAlgorithm,
    summary: &str,
    ctx: CommandContext,
) -> CommandResult {
    let args = &ctx.args;
    if args.iter().any(|a| a == "--help") {
        return CommandResult::success(make_help(name, summary));
    }

    let mut check = false;
    let mut files: Vec<String> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-c" | "--check" => check = true,
            "-b" | "-t" | "--binary" | "--text" => { /* ignored */ }
            a if a.starts_with('-') && a != "-" => {
                return CommandResult::with_exit_code(
                    "".into(),
                    format!("{}: invalid option -- '{}'\n", name, &a[1..2]),
                    1,
                );
            }
            _ => files.push(arg.clone()),
        }
    }
    if files.is_empty() {
        files.push("-".into());
    }

    if check {
        let mut failed = 0;
        let mut output = String::new();

        for file in &files {
            let content = if file == "-" {
                Some(ctx.stdin.clone())
            } else {
                let path = resolve_path(&ctx.cwd, file);
                ctx.fs.read_file(&path).await.ok()
            };
            let content = match content {
                Some(c) => c,
                None => {
                    return CommandResult::with_exit_code(
                        "".into(),
                        no_such_file(name, file),
                        1,
                    )
                }
            };

            for line in content.lines() {
                // Parse: hash  filename  or  hash *filename
                let parts: Vec<&str> = line.splitn(2, |c: char| c.is_whitespace()).collect();
                if parts.len() < 2 {
                    continue;
                }
                let expected_hash = parts[0];
                let target_file = parts[1].trim_start_matches(|c: char| c == ' ' || c == '*');
                if target_file.is_empty() {
                    continue;
                }

                let file_data = if target_file == "-" {
                    Some(ctx.stdin.as_bytes().to_vec())
                } else {
                    let path = resolve_path(&ctx.cwd, target_file);
                    ctx.fs.read_file_buffer(&path).await.ok()
                };

                match file_data {
                    None => {
                        output.push_str(&format!("{}: FAILED open or read\n", target_file));
                        failed += 1;
                    }
                    Some(data) => {
                        let hash = compute_hash(algorithm, &data);
                        let ok = hash == expected_hash.to_lowercase();
                        output.push_str(&format!(
                            "{}: {}\n",
                            target_file,
                            if ok { "OK" } else { "FAILED" }
                        ));
                        if !ok {
                            failed += 1;
                        }
                    }
                }
            }
        }

        if failed > 0 {
            let s = if failed > 1 { "s" } else { "" };
            output.push_str(&format!(
                "{}: WARNING: {} computed checksum{} did NOT match\n",
                name, failed, s
            ));
        }
        return CommandResult::with_exit_code(output, "".into(), if failed > 0 { 1 } else { 0 });
    }

    // Normal hash mode
    let mut output = String::new();
    let mut exit_code = 0;

    for file in &files {
        let data = if file == "-" {
            Some(ctx.stdin.as_bytes().to_vec())
        } else {
            let path = resolve_path(&ctx.cwd, file);
            ctx.fs.read_file_buffer(&path).await.ok()
        };
        match data {
            None => {
                output.push_str(&no_such_file(name, file));
                exit_code = 1;
            }
            Some(data) => {
                output.push_str(&format!("{}  {}\n", compute_hash(algorithm, &data), file));
            }
        }
    }

    CommandResult::with_exit_code(output, "".into(), exit_code)
}

fn resolve_path(cwd: &str, path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        let cwd = cwd.trim_end_matches('/');
        format!("{}/{}", cwd, path)
    }
}

// --- md5sum ---
pub struct Md5sumCommand;
#[async_trait]
impl Command for Md5sumCommand {
    fn name(&self) -> &'static str {
        "md5sum"
    }
    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        checksum_execute(
            "md5sum",
            HashAlgorithm::Md5,
            "compute MD5 message digest",
            ctx,
        )
        .await
    }
}

// --- sha1sum ---
pub struct Sha1sumCommand;
#[async_trait]
impl Command for Sha1sumCommand {
    fn name(&self) -> &'static str {
        "sha1sum"
    }
    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        checksum_execute(
            "sha1sum",
            HashAlgorithm::Sha1,
            "compute SHA1 message digest",
            ctx,
        )
        .await
    }
}

// --- sha256sum ---
pub struct Sha256sumCommand;
#[async_trait]
impl Command for Sha256sumCommand {
    fn name(&self) -> &'static str {
        "sha256sum"
    }
    async fn execute(&self, ctx: CommandContext) -> CommandResult {
        checksum_execute(
            "sha256sum",
            HashAlgorithm::Sha256,
            "compute SHA256 message digest",
            ctx,
        )
        .await
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
    async fn test_md5_hello() {
        let r = Md5sumCommand.execute(make_ctx(vec![], "hello")).await;
        assert_eq!(r.stdout, "5d41402abc4b2a76b9719d911017c592  -\n");
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_empty() {
        let r = Md5sumCommand.execute(make_ctx(vec![], "")).await;
        assert_eq!(r.stdout, "d41d8cd98f00b204e9800998ecf8427e  -\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_file() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/tmp/test.txt", "test".as_bytes())
            .await
            .unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["/tmp/test.txt"], "", fs))
            .await;
        assert_eq!(
            r.stdout,
            "098f6bcd4621d373cade4e832627b4f6  /tmp/test.txt\n"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_missing_file() {
        let r = Md5sumCommand
            .execute(make_ctx(vec!["/tmp/nonexistent"], ""))
            .await;
        assert!(r.stdout.contains("No such file or directory"));
        assert_eq!(r.exit_code, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_check_ok() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/tmp/hello.txt", "hello".as_bytes())
            .await
            .unwrap();
        fs.write_file(
            "/tmp/sums.txt",
            "5d41402abc4b2a76b9719d911017c592  /tmp/hello.txt\n".as_bytes(),
        )
        .await
        .unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["-c", "/tmp/sums.txt"], "", fs))
            .await;
        assert!(r.stdout.contains("/tmp/hello.txt: OK"));
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_check_fail() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/tmp/wrong.txt", "wrong".as_bytes())
            .await
            .unwrap();
        fs.write_file(
            "/tmp/sums.txt",
            "5d41402abc4b2a76b9719d911017c592  /tmp/wrong.txt\n".as_bytes(),
        )
        .await
        .unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["-c", "/tmp/sums.txt"], "", fs))
            .await;
        assert!(r.stdout.contains("/tmp/wrong.txt: FAILED"));
        assert!(r.stdout.contains("WARNING"));
        assert_eq!(r.exit_code, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_help() {
        let r = Md5sumCommand.execute(make_ctx(vec!["--help"], "")).await;
        assert!(r.stdout.contains("md5sum"));
        assert!(r.stdout.contains("MD5"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha1_hello() {
        let r = Sha1sumCommand.execute(make_ctx(vec![], "hello")).await;
        assert_eq!(r.stdout, "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d  -\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha1_empty() {
        let r = Sha1sumCommand.execute(make_ctx(vec![], "")).await;
        assert_eq!(r.stdout, "da39a3ee5e6b4b0d3255bfef95601890afd80709  -\n");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha256_hello() {
        let r = Sha256sumCommand.execute(make_ctx(vec![], "hello")).await;
        assert_eq!(
            r.stdout,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  -\n"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha256_empty() {
        let r = Sha256sumCommand.execute(make_ctx(vec![], "")).await;
        assert_eq!(
            r.stdout,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  -\n"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha256_help() {
        let r = Sha256sumCommand.execute(make_ctx(vec!["--help"], "")).await;
        assert!(r.stdout.contains("sha256sum"));
        assert!(r.stdout.contains("SHA256"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_multiple_files() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/tmp/a.txt", "a".as_bytes()).await.unwrap();
        fs.write_file("/tmp/b.txt", "b".as_bytes()).await.unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["/tmp/a.txt", "/tmp/b.txt"], "", fs))
            .await;
        assert!(r
            .stdout
            .contains("0cc175b9c0f1b6a831c399e269772661  /tmp/a.txt"));
        assert!(r
            .stdout
            .contains("92eb5ffee6ae2fec3ad71c777531578f  /tmp/b.txt"));
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_check_multiple_files() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/tmp/hello.txt", "hello".as_bytes())
            .await
            .unwrap();
        fs.write_file("/tmp/world.txt", "world".as_bytes())
            .await
            .unwrap();
        fs.write_file("/tmp/sums.txt", "5d41402abc4b2a76b9719d911017c592  /tmp/hello.txt\n7d793037a0760186574b0282f2f435e7  /tmp/world.txt\n".as_bytes()).await.unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["-c", "/tmp/sums.txt"], "", fs))
            .await;
        assert!(r.stdout.contains("/tmp/hello.txt: OK"));
        assert!(r.stdout.contains("/tmp/world.txt: OK"));
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_check_mixed_results() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/tmp/correct.txt", "hello".as_bytes())
            .await
            .unwrap();
        fs.write_file("/tmp/wrong.txt", "wrong".as_bytes())
            .await
            .unwrap();
        fs.write_file("/tmp/sums.txt", "5d41402abc4b2a76b9719d911017c592  /tmp/correct.txt\n5d41402abc4b2a76b9719d911017c592  /tmp/wrong.txt\n".as_bytes()).await.unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["-c", "/tmp/sums.txt"], "", fs))
            .await;
        assert!(r.stdout.contains("/tmp/correct.txt: OK"));
        assert!(r.stdout.contains("/tmp/wrong.txt: FAILED"));
        assert!(r.stdout.contains("WARNING"));
        assert!(r.stdout.contains("1 computed checksum"));
        assert_eq!(r.exit_code, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_binary_file_with_invalid_utf8() {
        let fs = Arc::new(InMemoryFs::new());
        // PNG magic bytes
        fs.write_file("/binary.dat", &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a])
            .await
            .unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["/binary.dat"], "", fs))
            .await;
        assert_eq!(r.stdout, "8eece9cc616084e69299f7f1a53a6404  /binary.dat\n");
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_binary_file_with_null_bytes() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/nulls.dat", &[0x00, 0x00, 0x00, 0x00])
            .await
            .unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["/nulls.dat"], "", fs))
            .await;
        assert_eq!(r.stdout, "f1d3ff8443297732862df21dc4e57262  /nulls.dat\n");
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha1_file() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/test.txt", "test".as_bytes()).await.unwrap();
        let r = Sha1sumCommand
            .execute(make_ctx_with_fs(vec!["/test.txt"], "", fs))
            .await;
        assert_eq!(
            r.stdout,
            "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3  /test.txt\n"
        );
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha256_file() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/test.txt", "test".as_bytes()).await.unwrap();
        let r = Sha256sumCommand
            .execute(make_ctx_with_fs(vec!["/test.txt"], "", fs))
            .await;
        assert_eq!(
            r.stdout,
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08  /test.txt\n"
        );
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha256_binary_file() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/binary.dat", &[0x89, 0x50, 0x4e, 0x47])
            .await
            .unwrap();
        let r = Sha256sumCommand
            .execute(make_ctx_with_fs(vec!["/binary.dat"], "", fs))
            .await;
        assert_eq!(
            r.stdout,
            "0f4636c78f65d3639ece5a064b5ae753e3408614a14fb18ab4d7540d2c248543  /binary.dat\n"
        );
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha1_check_ok() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/hello.txt", "hello".as_bytes())
            .await
            .unwrap();
        fs.write_file(
            "/sums.txt",
            "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d  /hello.txt\n".as_bytes(),
        )
        .await
        .unwrap();
        let r = Sha1sumCommand
            .execute(make_ctx_with_fs(vec!["-c", "/sums.txt"], "", fs))
            .await;
        assert!(r.stdout.contains("/hello.txt: OK"));
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha256_check_ok() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file("/hello.txt", "hello".as_bytes())
            .await
            .unwrap();
        fs.write_file(
            "/sums.txt",
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  /hello.txt\n"
                .as_bytes(),
        )
        .await
        .unwrap();
        let r = Sha256sumCommand
            .execute(make_ctx_with_fs(vec!["-c", "/sums.txt"], "", fs))
            .await;
        assert!(r.stdout.contains("/hello.txt: OK"));
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_check_missing_file() {
        let fs = Arc::new(InMemoryFs::new());
        fs.write_file(
            "/sums.txt",
            "5d41402abc4b2a76b9719d911017c592  /missing.txt\n".as_bytes(),
        )
        .await
        .unwrap();
        let r = Md5sumCommand
            .execute(make_ctx_with_fs(vec!["-c", "/sums.txt"], "", fs))
            .await;
        assert!(r.stdout.contains("/missing.txt: FAILED open or read"));
        assert!(r.stdout.contains("WARNING"));
        assert_eq!(r.exit_code, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_md5_stdin_dash() {
        let r = Md5sumCommand.execute(make_ctx(vec!["-"], "hello")).await;
        assert_eq!(r.stdout, "5d41402abc4b2a76b9719d911017c592  -\n");
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sha1_help() {
        let r = Sha1sumCommand.execute(make_ctx(vec!["--help"], "")).await;
        assert!(r.stdout.contains("sha1sum"));
        assert!(r.stdout.contains("SHA1"));
        assert_eq!(r.exit_code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_invalid_option() {
        let r = Md5sumCommand.execute(make_ctx(vec!["-z"], "")).await;
        assert!(r.stderr.contains("invalid option"));
        assert_eq!(r.exit_code, 1);
    }
}
