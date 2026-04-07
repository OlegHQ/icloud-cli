mod cmd_bash;
mod cmd_hme;
mod cmd_notes;
mod cmd_reminders;
mod output;

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use icloud_api::notes::{NotesStore, NotesSyncEngine};
use icloud_api::reminders::{RemindersStore, SyncEngine};
use icloud_api::session::{
    default_reminders_db_path, default_session_path, load_session, save_session, SecretsBackend,
};
use icloud_api::Result as IResult;
use icloud_api::{is_cache_fresh, AuthFlow};

use cmd_hme::HmeCmd;
use cmd_notes::NotesCmd;
use cmd_reminders::RemindersCmd;
use output::{hint, print_ok_with, print_whoami, OutputMode};

// ── Exit codes ─────────────────────────────────────────────

const EXIT_USAGE: i32 = 2;
const EXIT_AUTH: i32 = 3;
const EXIT_UPSTREAM: i32 = 4;

fn exit_code(e: &icloud_api::Error) -> i32 {
    match e {
        icloud_api::Error::Auth(_)
        | icloud_api::Error::Session(_)
        | icloud_api::Error::Keyring(_) => EXIT_AUTH,
        icloud_api::Error::Reminders(msg) | icloud_api::Error::Notes(msg)
            if msg.contains("not found")
                || msg.contains("no changes")
                || msg.contains("missing") =>
        {
            EXIT_USAGE
        }
        _ => EXIT_UPSTREAM,
    }
}

// ── Shared arg structs ────────────────────────────────────

#[derive(Args, Clone)]
pub(crate) struct SessionArg {
    /// Path to session file (default: XDG config dir).
    #[arg(long)]
    session: Option<PathBuf>,
}

#[derive(Args, Clone)]
pub(crate) struct RemindersArgs {
    #[command(flatten)]
    pub(crate) sess: SessionArg,
    /// Path to redb database file.
    #[arg(long)]
    db: Option<PathBuf>,
}

impl SessionArg {
    pub(crate) fn path(&self) -> PathBuf {
        self.session.clone().unwrap_or_else(default_session_path)
    }
}

#[derive(Args, Clone)]
pub(crate) struct NotesArgs {
    #[command(flatten)]
    pub(crate) sess: SessionArg,
    /// Path to notes database file.
    #[arg(long)]
    db: Option<PathBuf>,
}

impl NotesArgs {
    pub(crate) fn session_path(&self) -> PathBuf {
        self.sess.path()
    }
    pub(crate) fn db_path(&self) -> PathBuf {
        self.db
            .clone()
            .unwrap_or_else(icloud_api::session::default_notes_db_path)
    }
}

impl RemindersArgs {
    pub(crate) fn session_path(&self) -> PathBuf {
        self.sess.path()
    }
    pub(crate) fn db_path(&self) -> PathBuf {
        self.db.clone().unwrap_or_else(default_reminders_db_path)
    }
}

// ── CLI definition ─────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "icloud",
    version,
    about = "CLI for iCloud Reminders, Notes, and Hide My Email"
)]
struct Cli {
    /// Output JSON to stdout for scripts and agents.
    #[arg(
        long,
        global = true,
        alias = "json-output",
        alias = "jsonOutput",
        short = 'j'
    )]
    json: bool,

    /// Emit stable tab-separated output for piping.
    #[arg(long, global = true)]
    plain: bool,

    /// Minimal output (counts only).
    #[arg(long, global = true, short = 'q')]
    quiet: bool,

    /// Disable colored output.
    #[arg(long, global = true)]
    no_color: bool,

    /// Disable interactive prompts (fail instead of asking).
    #[arg(long, global = true)]
    no_input: bool,

    /// Session secret storage: plain JSON file or OS keychain.
    #[arg(long, global = true, value_enum, default_value_t = SecretsArg::File)]
    secrets: SecretsArg,

    /// Skip the network check if the local cache is younger than this many seconds (0 = always check).
    /// The check is lightweight (only fetches changes since last sync), so the default is low.
    #[arg(long, global = true, default_value = "5")]
    max_age: u64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Default, ValueEnum)]
enum SecretsArg {
    /// Full session in a JSON file (default, works in CI/headless).
    #[default]
    File,
    /// Public JSON on disk, secrets in OS keychain.
    Keychain,
}

impl From<SecretsArg> for SecretsBackend {
    fn from(a: SecretsArg) -> Self {
        match a {
            SecretsArg::File => SecretsBackend::File,
            SecretsArg::Keychain => SecretsBackend::Keychain,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Sign in with Apple ID (SRP + 2FA) and persist the session.
    Login {
        /// Apple ID email address.
        #[arg(long, env = "ICLOUD_USERNAME")]
        username: Option<String>,
        /// Apple ID password.
        #[arg(long, env = "ICLOUD_PASSWORD")]
        password: Option<String>,
        /// 2FA code (skips interactive prompt).
        #[arg(long, env = "ICLOUD_2FA_CODE")]
        code: Option<String>,
        #[command(flatten)]
        sess: SessionArg,
    },
    /// Validate the current session and show account info.
    Whoami {
        #[command(flatten)]
        sess: SessionArg,
    },
    /// Manage iCloud Reminders.
    #[command(subcommand)]
    Reminders(RemindersCmd),
    /// Manage iCloud Notes.
    #[command(subcommand)]
    Notes(NotesCmd),
    /// Manage Hide My Email aliases (iCloud+ required).
    #[command(subcommand)]
    Hme(HmeCmd),
    /// Run bash (bashbox) against the iCloud virtual filesystem.
    Bash {
        #[command(flatten)]
        sess: SessionArg,
        /// Notes database path.
        #[arg(long)]
        notes_db: Option<PathBuf>,
        /// Reminders database path.
        #[arg(long)]
        reminders_db: Option<PathBuf>,
        /// Execute a one-line script (same as `bash -c`).
        #[arg(short = 'c', long)]
        command: Option<String>,
        /// Optional script file (use `-` for stdin when no `-c`).
        #[arg(value_name = "SCRIPT")]
        script: Option<PathBuf>,
    },
}

// ── Helpers ────────────────────────────────────────────────

pub(crate) fn sync_spinner(label: &str, suppress: bool) -> Option<indicatif::ProgressBar> {
    if suppress {
        return None;
    }
    let sp = indicatif::ProgressBar::new_spinner();
    sp.set_style(
        indicatif::ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    sp.set_message(format!("Syncing {label}..."));
    sp.enable_steady_tick(Duration::from_millis(80));
    Some(sp)
}

macro_rules! open_service {
    (
        $name:ident,
        store: $store:ty,
        engine: $engine:ty,
        ck_ctor: $ck_ctor:path,
        db_err: $db_err:path,
        label: $label:expr,
    ) => {
        pub(crate) struct $name {
            pub(crate) store: $store,
            pub(crate) engine: $engine,
        }

        impl $name {
            pub(crate) async fn open(
                session_path: &std::path::Path,
                db_path: &std::path::Path,
                secrets: SecretsBackend,
                force: bool,
                max_age: u64,
                suppress_spinner: bool,
            ) -> IResult<Self> {
                let s = load_session(session_path, secrets)?;
                let ck = $ck_ctor(s)?;
                let store = <$store>::new(db_path);
                if force {
                    store.remove_db_file().map_err(|e| $db_err(e.to_string()))?;
                }
                let cache = store.load_cache()?;
                let fresh = !force && is_cache_fresh(&cache, max_age);
                let mut engine = <$engine>::new(ck, cache);

                if !fresh {
                    let spinner = sync_spinner($label, suppress_spinner);
                    let result = engine.sync(force).await;
                    if let Some(sp) = spinner {
                        sp.finish_and_clear();
                    }
                    result?;
                }

                Ok(Self { store, engine })
            }

            pub(crate) fn save(&mut self) -> IResult<()> {
                self.store.save_cache(&mut self.engine.cache)
            }
        }
    };
}

open_service!(
    OpenReminders,
    store: RemindersStore,
    engine: SyncEngine,
    ck_ctor: icloud_api::CloudKitClient::reminders,
    db_err: icloud_api::Error::Reminders,
    label: "reminders",
);

open_service!(
    OpenNotes,
    store: NotesStore,
    engine: NotesSyncEngine,
    ck_ctor: icloud_api::CloudKitClient::notes,
    db_err: icloud_api::Error::Notes,
    label: "notes",
);

async fn prompt_2fa(info: &icloud_api::TwoFactorInfo, auth: &AuthFlow) -> IResult<String> {
    if !info.trusted_phones.is_empty() {
        let phone = &info.trusted_phones[0];
        eprintln!("Requesting SMS code to {}...", phone.number);
        auth.request_sms_code(phone.id).await?;
    } else {
        eprintln!("2FA required — check your trusted devices for a code.");
    }
    eprint!("Enter {}-digit code: ", info.code_length);
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| icloud_api::Error::Auth(format!("failed to read input: {e}")))?;
    let input = input.trim().to_string();
    if input.is_empty() {
        return Err(icloud_api::Error::Auth("no 2FA code entered".into()));
    }
    Ok(input)
}

// ── Due date resolution (natural language → YYYY-MM-DD) ──

pub(crate) fn resolve_due_date(input: &str) -> String {
    let today = chrono::Local::now().date_naive();
    match input.to_ascii_lowercase().as_str() {
        "today" => today.format("%Y-%m-%d").to_string(),
        "tomorrow" => (today + chrono::Days::new(1))
            .format("%Y-%m-%d")
            .to_string(),
        "yesterday" => (today - chrono::Days::new(1))
            .format("%Y-%m-%d")
            .to_string(),
        _ => input.to_string(),
    }
}

// ── Smart date filter resolution ──────────────────────────

pub(crate) fn resolve_date_filter(
    filter: Option<&str>,
    all: bool,
) -> (bool, Option<String>, Option<String>) {
    if all {
        return (true, None, None);
    }
    let Some(f) = filter else {
        return (false, None, None);
    };
    let today = chrono::Local::now().date_naive();
    match f.to_ascii_lowercase().as_str() {
        "today" | "tday" => {
            // Today + overdue (everything due up to today)
            let d = today.format("%Y-%m-%d").to_string();
            (false, None, Some(d))
        }
        "tomorrow" | "t" => {
            let d = (today + chrono::Days::new(1))
                .format("%Y-%m-%d")
                .to_string();
            (false, Some(d.clone()), Some(d))
        }
        "week" | "w" => {
            let from = today.format("%Y-%m-%d").to_string();
            let to = (today + chrono::Days::new(7))
                .format("%Y-%m-%d")
                .to_string();
            (false, Some(from), Some(to))
        }
        "overdue" | "o" => {
            let to = (today - chrono::Days::new(1))
                .format("%Y-%m-%d")
                .to_string();
            (false, None, Some(to))
        }
        "upcoming" | "u" => {
            // All incomplete with a due date (no date bounds, just filter for has-due)
            (false, Some("0000-01-01".to_string()), None)
        }
        "completed" | "done" | "c" => (true, None, None),
        "all" | "a" => (true, None, None),
        date_str => {
            // Treat as a specific date
            (
                false,
                Some(date_str.to_string()),
                Some(date_str.to_string()),
            )
        }
    }
}

pub(crate) fn read_body_or_stdin(body: Option<String>) -> IResult<String> {
    if let Some(b) = body {
        return Ok(b);
    }
    let mut buf = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
        .map_err(|e| icloud_api::Error::Notes(format!("stdin: {e}")))?;
    Ok(buf)
}

// ── Entry point ────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let out = OutputMode {
        json: cli.json,
        plain: cli.plain,
        quiet: cli.quiet,
        no_color: cli.no_color,
        no_input: cli.no_input,
    };
    let is_json = out.json;
    if let Err(e) = run(cli, out).await {
        let code = exit_code(&e);
        if is_json {
            let r = e.json_report();
            eprintln!("{}", serde_json::to_string(&r).unwrap());
        } else {
            eprintln!("error: {e}");
            if let Some(h) = e.json_report().hint {
                eprintln!("hint: {h}");
            }
        }
        std::process::exit(code);
    }
}

// ── Command dispatch ───────────────────────────────────────

async fn run(cli: Cli, out: OutputMode) -> IResult<()> {
    let json = out.json;
    let secrets: SecretsBackend = cli.secrets.into();
    let max_age = cli.max_age;

    match cli.command {
        Command::Login {
            username,
            password,
            code,
            sess,
        } => handle_login(out, secrets, username, password, code, sess).await,
        Command::Whoami { sess } => handle_whoami(out, secrets, sess).await,
        Command::Reminders(sub) => {
            cmd_reminders::handle_reminders(out, secrets, max_age, sub).await
        }
        Command::Notes(sub) => cmd_notes::handle_notes(out, secrets, max_age, sub).await,
        Command::Hme(sub) => cmd_hme::handle_hme(json, secrets, sub).await,
        Command::Bash {
            sess,
            notes_db,
            reminders_db,
            command,
            script,
        } => {
            cmd_bash::run_bash(
                secrets,
                max_age,
                sess,
                notes_db,
                reminders_db,
                command,
                script,
            )
            .await
        }
    }
}

// ── Handlers ──────────────────────────────────────────────

async fn handle_login(
    out: OutputMode,
    secrets: SecretsBackend,
    username: Option<String>,
    password: Option<String>,
    code: Option<String>,
    sess: SessionArg,
) -> IResult<()> {
    let json = out.json;
    let user = username
        .ok_or_else(|| icloud_api::Error::Auth("missing --username or ICLOUD_USERNAME".into()))?;
    let pass = password
        .ok_or_else(|| icloud_api::Error::Auth("missing --password or ICLOUD_PASSWORD".into()))?;
    let session_path = sess.path();
    let mut auth = AuthFlow::new(user, pass);
    let tfa_info = auth.login_srp().await?;
    if let Some(info) = tfa_info {
        if let Some(c) = code {
            auth.submit_2fa(&c).await?;
        } else if !json && !out.no_input {
            let code = prompt_2fa(&info, &auth).await?;
            if let Some(phone) = info.trusted_phones.first() {
                auth.submit_sms_code(&code, phone.id).await?;
            } else {
                auth.submit_2fa(&code).await?;
            }
        } else {
            return Err(icloud_api::Error::Auth(
                "two-factor authentication required (pass --code or ICLOUD_2FA_CODE)".into(),
            ));
        };
    }
    let data = auth.finish_login().await?;
    save_session(&session_path, &data, secrets)?;
    print_ok_with(
        json,
        &format!("Logged in. Session saved to {}", session_path.display()),
        &serde_json::json!({"session": session_path.display().to_string(), "ck_base_url": data.ck_base_url}),
    );
    if out.is_human() {
        hint(&[
            "icloud whoami              — verify session",
            "icloud reminders sync      — sync reminders",
            "icloud reminders list      — list reminders",
        ]);
    }
    Ok(())
}

async fn handle_whoami(out: OutputMode, secrets: SecretsBackend, sess: SessionArg) -> IResult<()> {
    let json = out.json;
    let session_path = sess.path();
    let session_data = load_session(&session_path, secrets)?;
    let ok = AuthFlow::validate_session(&session_data)
        .await
        .unwrap_or(false);
    print_whoami(json, &session_path, &session_data, ok);
    if out.is_human() && !ok {
        hint(&["icloud login               — re-authenticate"]);
    }
    Ok(())
}
