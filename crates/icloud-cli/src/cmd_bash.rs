//! `icloud bash` — run bashbox against the iCloud VFS.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use bashbox::bash::BashOptions;
use bashbox::{Bash, InMemoryFs};
use icloud_api::hme::HideMyEmailClient;
use icloud_api::session::{load_session, SecretsBackend};
use icloud_bash::ICloudFs;

use crate::read_body_or_stdin;
use crate::{IcloudFsArgs, OpenNotes, OpenReminders};

pub async fn run_bash(
    secrets: SecretsBackend,
    max_age: u64,
    args: IcloudFsArgs,
    command: Option<String>,
    script: Option<PathBuf>,
) -> icloud_api::Result<()> {
    let session_path = args.session_path();
    let ndp = args.notes_db_path();
    let rdp = args.reminders_db_path();

    let notes = OpenNotes::open(&session_path, &ndp, secrets, false, max_age, true).await?;
    let reminders = OpenReminders::open(&session_path, &rdp, secrets, false, max_age, true).await?;

    let session = load_session(&session_path, secrets)?;
    let hme = HideMyEmailClient::new(session)?;

    let OpenNotes {
        store: ns,
        engine: ne,
    } = notes;
    let OpenReminders {
        store: rs,
        engine: re,
    } = reminders;

    let ne = Arc::new(tokio::sync::Mutex::new(ne));
    let re = Arc::new(tokio::sync::Mutex::new(re));

    let inner = Arc::new(InMemoryFs::new());
    // Keep a typed handle alongside the trait-object one so the REPL can
    // call ICloudFs-specific methods (cache invalidation around `edit`).
    let icloud_fs_typed =
        Arc::new(ICloudFs::new(inner, ne.clone(), re.clone(), Arc::new(hme)).await);
    let icloud_fs: Arc<dyn bashbox::fs::FileSystem> = icloud_fs_typed.clone();

    let (n_count, r_count) = {
        let n = ne.lock().await;
        let r = re.lock().await;
        (n.cache.notes.len(), r.cache.reminders.len())
    };

    let mut env = HashMap::new();
    env.insert("HOME".to_string(), "/".to_string());
    env.insert("USER".to_string(), "icloud".to_string());
    env.insert(
        "ICLOUD_SESSION".to_string(),
        session_path.display().to_string(),
    );
    env.insert("ICLOUD_NOTES_COUNT".to_string(), n_count.to_string());
    env.insert("ICLOUD_REMINDERS_COUNT".to_string(), r_count.to_string());

    let mut bash = Bash::new(BashOptions {
        fs: Some(icloud_fs),
        cwd: Some("/".to_string()),
        env: Some(env),
        limits: None,
    })
    .await;

    let script_text = if let Some(c) = command {
        Some(c)
    } else if let Some(p) = script {
        if p.as_os_str() == "-" {
            Some(read_body_or_stdin(None)?)
        } else {
            Some(std::fs::read_to_string(&p).map_err(icloud_api::Error::Io)?)
        }
    } else if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        None // interactive mode
    } else {
        Some(read_body_or_stdin(None)?)
    };

    let exit_code = if let Some(script_text) = script_text {
        let res = bash.exec(&script_text, None).await;
        print!("{}", res.stdout);
        eprint!("{}", res.stderr);
        res.exit_code
    } else {
        run_repl(&mut bash, &icloud_fs_typed).await
    };

    {
        let mut n = ne.lock().await;
        ns.save_cache(&mut n.cache)?;
    }
    {
        let mut r = re.lock().await;
        rs.save_cache(&mut r.cache)?;
    }

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

/// Result of dispatching a REPL line to a builtin.
enum BuiltinOutcome {
    /// Builtin handled the line; REPL should continue reading.
    Handled,
    /// Builtin asked the REPL to exit with the given code.
    Exit(i32),
    /// Not a builtin — fall through to `bash.exec`.
    NotHandled,
}

async fn run_repl(bash: &mut Bash, icloud_fs: &ICloudFs) -> i32 {
    use rustyline::error::ReadlineError;
    use rustyline::{Config, Editor};

    let config = Config::builder()
        .auto_add_history(true)
        .completion_type(rustyline::CompletionType::List)
        .build();
    let helper = VfsHelper::new();
    let mut rl: Editor<VfsHelper, rustyline::history::DefaultHistory> =
        match Editor::with_config(config) {
            Ok(rl) => rl,
            Err(e) => {
                eprintln!("readline init error: {e}");
                return 1;
            }
        };
    rl.set_helper(Some(helper));

    let history_path = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".icloud_bash_history"));
    if let Some(ref p) = history_path {
        let _ = rl.load_history(p);
    }

    let mut buf = String::new();
    loop {
        let prompt = if buf.is_empty() {
            format!("icloud:{}$ ", bash.get_cwd())
        } else {
            "> ".to_string()
        };

        // Refresh completion entries for the current cwd before each prompt.
        if buf.is_empty() {
            let cwd = bash.get_cwd().to_string();
            let entries = bash.fs.readdir(&cwd).await.unwrap_or_default();
            if let Some(h) = rl.helper_mut() {
                h.update(cwd, entries);
            }
        }

        match rl.readline(&prompt) {
            Ok(line) => {
                buf.push_str(&line);
                buf.push('\n');

                if is_incomplete(&buf) {
                    continue;
                }

                let input = std::mem::take(&mut buf);
                let trimmed = input.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match handle_repl_builtin(bash, icloud_fs, trimmed).await {
                    BuiltinOutcome::Exit(code) => {
                        if let Some(ref p) = history_path {
                            let _ = rl.save_history(p);
                        }
                        return code;
                    }
                    BuiltinOutcome::Handled => continue,
                    BuiltinOutcome::NotHandled => {}
                }
                let res = bash.exec(trimmed, None).await;
                if !res.stdout.is_empty() {
                    print!("{}", res.stdout);
                }
                if !res.stderr.is_empty() {
                    eprint!("{}", res.stderr);
                }
            }
            Err(ReadlineError::Interrupted) => {
                buf.clear();
                println!("^C");
            }
            Err(ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("readline error: {e}");
                return 1;
            }
        }
    }

    if let Some(ref p) = history_path {
        let _ = rl.save_history(p);
    }

    0
}

async fn handle_repl_builtin(bash: &mut Bash, icloud_fs: &ICloudFs, input: &str) -> BuiltinOutcome {
    let Ok(tokens) = shell_words::split(input) else {
        return BuiltinOutcome::NotHandled;
    };
    let Some(cmd) = tokens.first() else {
        return BuiltinOutcome::NotHandled;
    };

    match cmd.as_str() {
        "exit" => {
            if tokens.len() > 2 {
                eprintln!("exit: too many arguments");
                return BuiltinOutcome::Handled;
            }
            match tokens.get(1) {
                Some(value) => match value.parse::<i32>() {
                    Ok(code) => BuiltinOutcome::Exit(code),
                    Err(_) => {
                        eprintln!("exit: numeric argument required");
                        BuiltinOutcome::Exit(2)
                    }
                },
                None => BuiltinOutcome::Exit(0),
            }
        }
        "edit" => {
            if tokens.len() != 2 {
                eprintln!("edit: expected exactly one path");
                return BuiltinOutcome::Handled;
            }
            if let Err(e) = edit_vfs_file(bash, icloud_fs, &tokens[1]).await {
                eprintln!("edit: {e}");
            }
            BuiltinOutcome::Handled
        }
        _ => BuiltinOutcome::NotHandled,
    }
}

async fn edit_vfs_file(
    bash: &mut Bash,
    icloud_fs: &ICloudFs,
    path: &str,
) -> icloud_api::Result<()> {
    // Drop any stale cached body so the `read_file` below goes back to
    // CloudKit — otherwise editing a note that was modified on another
    // device (Apple Notes app, another iCloud bash session) within the
    // 5-minute cache TTL would show the wrong content.
    icloud_fs.invalidate_note_body_at(path).await;

    let original = match bash.read_file(path).await {
        Ok(content) => content,
        Err(bashbox::fs::types::FsError::NotFound { .. }) => String::new(),
        Err(err) => {
            return Err(icloud_api::Error::Usage(format!(
                "failed to read {path}: {err}"
            )));
        }
    };

    let mut temp = tempfile::NamedTempFile::new().map_err(icloud_api::Error::Io)?;
    temp.write_all(original.as_bytes())
        .map_err(icloud_api::Error::Io)?;
    temp.flush().map_err(icloud_api::Error::Io)?;

    let Some((editor, editor_args)) = editor_command() else {
        return Err(icloud_api::Error::Usage(
            "no editor found; set EDITOR or VISUAL".into(),
        ));
    };

    let status = Command::new(&editor)
        .args(&editor_args)
        .arg(temp.path())
        .status()
        .map_err(icloud_api::Error::Io)?;
    if !status.success() {
        return Ok(());
    }

    let edited = fs::read_to_string(temp.path()).map_err(icloud_api::Error::Io)?;
    if edited != original {
        bash.write_file(path, &edited)
            .await
            .map_err(|err| icloud_api::Error::Usage(format!("failed to write {path}: {err}")))?;
        // After a successful write, drop the optimistic cached copy so the
        // next read re-fetches whatever Apple Notes / CloudKit actually
        // stored. The server may normalize blank lines, checklist runs,
        // etc. differently from what we sent.
        icloud_fs.invalidate_note_body_at(path).await;
    }

    Ok(())
}

fn editor_command() -> Option<(String, Vec<String>)> {
    for key in ["VISUAL", "EDITOR"] {
        if let Ok(value) = std::env::var(key) {
            if let Ok(mut parts) = shell_words::split(&value) {
                if let Some(program) = parts.first().cloned() {
                    parts.remove(0);
                    return Some((program, parts));
                }
            }
        }
    }

    Some(("vi".to_string(), Vec::new()))
}

/// Rustyline helper that completes the last word of a line against the
/// entries of the shell's current working directory, and against known
/// builtin names when the cursor sits on the command position.
///
/// The entry list is refreshed by the REPL loop before each prompt via
/// [`VfsHelper::update`], so we stay sync inside the `Completer` impl.
#[derive(Default)]
struct VfsHelper {
    cwd: String,
    entries: Vec<String>,
}

impl VfsHelper {
    fn new() -> Self {
        Self::default()
    }

    fn update(&mut self, cwd: String, mut entries: Vec<String>) {
        entries.sort();
        self.cwd = cwd;
        self.entries = entries;
    }
}

const REPL_BUILTINS: &[&str] = &["edit", "exit"];

impl rustyline::completion::Completer for VfsHelper {
    type Candidate = rustyline::completion::Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let (start, word) = word_at(line, pos);
        // If we're completing the first token of the line, offer builtins
        // alongside entries from cwd.
        let is_command_position = line[..start].trim().is_empty();

        let mut candidates: Vec<rustyline::completion::Pair> = Vec::new();
        if is_command_position {
            for name in REPL_BUILTINS {
                if name.starts_with(word) {
                    candidates.push(rustyline::completion::Pair {
                        display: (*name).to_string(),
                        replacement: format!("{name} "),
                    });
                }
            }
        }
        for entry in &self.entries {
            if entry.starts_with(word) {
                let replacement = if needs_shell_quoting(entry) {
                    shell_quote(entry)
                } else {
                    entry.clone()
                };
                candidates.push(rustyline::completion::Pair {
                    display: entry.clone(),
                    replacement,
                });
            }
        }
        Ok((start, candidates))
    }
}

impl rustyline::hint::Hinter for VfsHelper {
    type Hint = String;
}
impl rustyline::highlight::Highlighter for VfsHelper {}
impl rustyline::validate::Validator for VfsHelper {}
impl rustyline::Helper for VfsHelper {}

fn word_at(line: &str, pos: usize) -> (usize, &str) {
    let upto = &line[..pos];
    let start = upto
        .rfind(|c: char| c.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    (start, &line[start..pos])
}

fn needs_shell_quoting(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '('
                | ')'
                | '\''
                | '"'
                | '\\'
                | '|'
                | '&'
                | ';'
                | '<'
                | '>'
                | '$'
                | '`'
                | '*'
                | '?'
                | '['
                | ']'
        )
    })
}

fn shell_quote(s: &str) -> String {
    // Wrap in single quotes and escape any embedded single quote as '"'"'.
    let escaped = s.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}

fn is_incomplete(input: &str) -> bool {
    let trimmed = input.trim_end();
    if trimmed.ends_with('\\') {
        return true;
    }
    // Check unmatched quotes
    let mut single = false;
    let mut double = false;
    let mut escaped = false;
    for ch in trimmed.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '\'' && !double {
            single = !single;
        }
        if ch == '"' && !single {
            double = !double;
        }
    }
    single || double
}
