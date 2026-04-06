//! Optional split persistence: public JSON on disk + secrets in the OS keychain (`keyring`).

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::{Error, Result};
use super::{SessionCookie, SessionData};

/// Non-sensitive fields written to the session path when using [`SecretsBackend::Keychain`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionPublic {
    pub apple_id: String,
    pub ck_base_url: String,
    #[serde(default)]
    pub dsid: Option<String>,
    #[serde(default)]
    pub webservices: Option<serde_json::Value>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub account_country: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// Sensitive session fields stored in the keychain as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSecrets {
    #[serde(default)]
    pub session_token: Option<String>,
    #[serde(default)]
    pub trust_token: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub scnt: Option<String>,
    #[serde(default)]
    pub cookies: Vec<SessionCookie>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretsBackend {
    /// Full `SessionData` JSON at `path` (CI / headless).
    File,
    /// `SessionPublic` at `path`; secrets under keyring service `icloud-cli`, user `session:{apple_id}`.
    Keychain,
}

impl SessionPublic {
    pub fn from_session(data: &SessionData, apple_id: impl Into<String>) -> Self {
        Self {
            apple_id: apple_id.into(),
            ck_base_url: data.ck_base_url.clone(),
            dsid: data.dsid.clone(),
            webservices: data.webservices.clone(),
            client_id: data.client_id.clone(),
            account_country: data.account_country.clone(),
            created_at: data.created_at.clone(),
        }
    }
}

impl SessionSecrets {
    pub fn from_session(data: &SessionData) -> Self {
        Self {
            session_token: data.session_token.clone(),
            trust_token: data.trust_token.clone(),
            session_id: data.session_id.clone(),
            scnt: data.scnt.clone(),
            cookies: data.cookies.clone(),
        }
    }
}

impl SessionData {
    pub fn merge_public_secrets(public: SessionPublic, secrets: SessionSecrets) -> Self {
        SessionData {
            ck_base_url: public.ck_base_url,
            session_token: secrets.session_token,
            trust_token: secrets.trust_token,
            account_country: public.account_country,
            session_id: secrets.session_id,
            scnt: secrets.scnt,
            dsid: public.dsid,
            cookies: secrets.cookies,
            created_at: public.created_at,
            webservices: public.webservices,
            client_id: public.client_id,
            apple_id: Some(public.apple_id),
        }
    }
}

fn keyring_entry(apple_id: &str) -> Result<keyring::Entry> {
    keyring::Entry::new("icloud-cli", &format!("session:{apple_id}"))
        .map_err(|e| Error::Keyring(e.to_string()))
}

/// Load session according to backend.
pub fn load_session(path: &Path, backend: SecretsBackend) -> Result<SessionData> {
    match backend {
        SecretsBackend::File => SessionData::load(path),
        SecretsBackend::Keychain => {
            let raw = std::fs::read_to_string(path).map_err(|e| Error::Session(e.to_string()))?;
            let public: SessionPublic =
                serde_json::from_str(&raw).map_err(|e| Error::Session(e.to_string()))?;
            let entry = keyring_entry(&public.apple_id)?;
            let secret_json = entry
                .get_password()
                .map_err(|e| Error::Keyring(e.to_string()))?;
            let secrets: SessionSecrets =
                serde_json::from_str(&secret_json).map_err(|e| Error::Session(e.to_string()))?;
            Ok(SessionData::merge_public_secrets(public, secrets))
        }
    }
}

/// Persist session. For keychain mode, `apple_id` must match the login Apple ID (stored in public file).
pub fn save_session(path: &Path, data: &SessionData, backend: SecretsBackend) -> Result<()> {
    match backend {
        SecretsBackend::File => SessionData::save(data, path),
        SecretsBackend::Keychain => {
            let apple_id = data
                .apple_id
                .clone()
                .ok_or_else(|| Error::Session("keychain mode requires apple_id on session (re-login)".into()))?;
            let public = SessionPublic::from_session(data, apple_id.clone());
            let secrets = SessionSecrets::from_session(data);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::Session(e.to_string()))?;
            }
            let pub_raw =
                serde_json::to_string_pretty(&public).map_err(|e| Error::Session(e.to_string()))?;
            std::fs::write(path, pub_raw).map_err(|e| Error::Session(e.to_string()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
            }
            let entry = keyring_entry(&apple_id)?;
            let sec_raw =
                serde_json::to_string(&secrets).map_err(|e| Error::Session(e.to_string()))?;
            entry
                .set_password(&sec_raw)
                .map_err(|e| Error::Keyring(e.to_string()))?;
            Ok(())
        }
    }
}
