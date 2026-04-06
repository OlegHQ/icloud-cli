use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Serialize)]
pub struct JsonErrorReport {
    pub kind: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("auth: {0}")]
    Auth(String),
    #[error("session: {0}")]
    Session(String),
    #[error("keychain: {0}")]
    Keyring(String),
    #[error("reminders: {0}")]
    Reminders(String),
    #[error("notes: {0}")]
    Notes(String),
    #[error("hide my email: {0}")]
    HideMyEmail(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid url: {0}")]
    Url(#[from] url::ParseError),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Construct the right error variant by label string ("reminders", "notes", …).
    pub(crate) fn from_label(label: &str, msg: String) -> Self {
        match label {
            "reminders" => Error::Reminders(msg),
            "notes" => Error::Notes(msg),
            _ => Error::Session(msg),
        }
    }

    /// Stable `{ kind, message, hint? }` for CLI `--json` (no secrets).
    pub fn json_report(&self) -> JsonErrorReport {
        let hint: Option<String> = match self {
            Error::Session(msg) if msg.contains("keychain") => {
                Some("Run `icloud login` with `--secrets file` or fix keychain access.".into())
            }
            Error::Api {
                status: 401 | 403, ..
            }
            | Error::Auth(_) => Some("Session may have expired; run `icloud login` again.".into()),
            Error::Reminders(msg) if msg.contains("schema") || msg.contains("incomplete") => Some(
                "Remove the reminders DB file and run `icloud reminders sync` (or `sync --force`)."
                    .into(),
            ),
            _ => None,
        };
        let (kind, message) = match self {
            Error::Http(e) => ("http", e.to_string()),
            Error::Api { status, body } => ("api", format!("HTTP {status}: {body}")),
            Error::Json(e) => ("json", e.to_string()),
            Error::Auth(m) => ("auth", m.clone()),
            Error::Session(m) => ("session", m.clone()),
            Error::Keyring(m) => ("keyring", m.clone()),
            Error::Reminders(m) => ("reminders", m.clone()),
            Error::Notes(m) => ("notes", m.clone()),
            Error::HideMyEmail(m) => ("hide_my_email", m.clone()),
            Error::Io(e) => ("io", e.to_string()),
            Error::Url(e) => ("url", e.to_string()),
        };
        JsonErrorReport {
            kind,
            message,
            hint,
        }
    }
}
