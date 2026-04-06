//! `icloud bash` — run just-bash against the iCloud VFS.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use icloud_api::hme::HideMyEmailClient;
use icloud_api::session::{
    default_notes_db_path, default_reminders_db_path, load_session, SecretsBackend,
};
use icloud_bash::ICloudFs;
use just_bash::bash::BashOptions;
use just_bash::{Bash, InMemoryFs};

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
        c
    } else if let Some(p) = script {
        if p.as_os_str() == "-" {
            read_body_or_stdin(None)?
        } else {
            std::fs::read_to_string(&p).map_err(icloud_api::Error::Io)?
        }
    } else {
        read_body_or_stdin(None)?
    };

    let res = bash.exec(&script_text, None).await;
    print!("{}", res.stdout);
    eprint!("{}", res.stderr);

    {
        let mut n = ne.lock().await;
        ns.save_cache(&mut n.cache)?;
    }
    {
        let mut r = re.lock().await;
        rs.save_cache(&mut r.cache)?;
    }

    if res.exit_code != 0 {
        std::process::exit(res.exit_code);
    }

    Ok(())
}
