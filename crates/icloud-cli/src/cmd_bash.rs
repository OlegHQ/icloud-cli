//! `icloud bash` — run bashbox against the iCloud VFS.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use icloud_api::hme::HideMyEmailClient;
use icloud_api::session::{
    default_notes_db_path, default_reminders_db_path, load_session, SecretsBackend,
};
use icloud_bash::ICloudFs;
use bashbox::bash::BashOptions;
use bashbox::{Bash, InMemoryFs};

use crate::read_body_or_stdin;
use crate::{OpenNotes, OpenReminders, SessionArg};

pub async fn run_bash(
    secrets: SecretsBackend,
    max_age: u64,
    sess: SessionArg,
    notes_db: Option<PathBuf>,
    reminders_db: Option<PathBuf>,
    command: Option<String>,
    script: Option<PathBuf>,
) -> icloud_api::Result<()> {
    let session_path = sess.path();
    let ndp = notes_db.unwrap_or_else(default_notes_db_path);
    let rdp = reminders_db.unwrap_or_else(default_reminders_db_path);

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
    let icloud_fs = Arc::new(ICloudFs::new(inner, ne.clone(), re.clone(), Arc::new(hme)));

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
        run_repl(&mut bash).await;
        0
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

async fn run_repl(bash: &mut Bash) {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;

    let mut rl = match DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => {
            eprintln!("readline init error: {e}");
            return;
        }
    };

    let history_path =
        std::env::var("HOME").ok().map(|h| std::path::PathBuf::from(h).join(".icloud_bash_history"));
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

                let _ = rl.add_history_entry(trimmed);
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
                break;
            }
        }
    }

    if let Some(ref p) = history_path {
        let _ = rl.save_history(p);
    }
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
