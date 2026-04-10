mod persist;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

pub use persist::{load_session, save_session, SecretsBackend, SessionPublic, SessionSecrets};

/// Persisted authentication state — compatible with
/// [tarekbecker/icloud-reminders-cli](https://github.com/tarekbecker/icloud-reminders-cli) `session.json`
/// plus optional `webservices` for Hide My Email and Notes query URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub ck_base_url: String,
    #[serde(default)]
    pub session_token: Option<String>,
    #[serde(default)]
    pub trust_token: Option<String>,
    #[serde(default)]
    pub account_country: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub scnt: Option<String>,
    #[serde(default)]
    pub dsid: Option<String>,
    #[serde(default)]
    pub cookies: Vec<SessionCookie>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub webservices: Option<serde_json::Value>,
    #[serde(default)]
    pub client_id: Option<String>,
    /// Apple ID used for this session (required for keychain-backed secret storage).
    #[serde(default)]
    pub apple_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: i64,
    pub secure: bool,
}

impl SessionData {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read_to_string(path).map_err(|e| Error::Session(e.to_string()))?;
        serde_json::from_str(&data).map_err(|e| Error::Session(e.to_string()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Session(e.to_string()))?;
        }
        let data = serde_json::to_string_pretty(self).map_err(|e| Error::Session(e.to_string()))?;
        std::fs::write(path, data).map_err(|e| Error::Session(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("com", "icloud-cli", "icloud-cli")
}

pub fn default_session_path() -> PathBuf {
    project_dirs()
        .map(|d| d.config_dir().join("session.json"))
        .unwrap_or_else(|| PathBuf::from("./session.json"))
}

/// Default path for the redb reminders database (XDG data dir).
pub fn default_reminders_db_path() -> PathBuf {
    project_dirs()
        .map(|d| d.data_dir().join("reminders.redb"))
        .unwrap_or_else(|| PathBuf::from("./reminders.redb"))
}

/// Default path for the redb notes database (XDG data dir).
pub fn default_notes_db_path() -> PathBuf {
    project_dirs()
        .map(|d| d.data_dir().join("notes.redb"))
        .unwrap_or_else(|| PathBuf::from("./notes.redb"))
}

/// Default path for the search index directory (XDG data dir).
pub fn default_search_index_path() -> PathBuf {
    project_dirs()
        .map(|d| d.data_dir().join("search-index"))
        .unwrap_or_else(|| PathBuf::from("./search-index"))
}

pub fn unquote_cookie_value(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}
