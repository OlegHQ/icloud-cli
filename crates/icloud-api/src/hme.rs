//! Hide My Email (`premiummailsettings` webservice).
//! Ported from [glizzykingdreko/icloud-hme](https://github.com/glizzykingdreko/icloud-hme).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::{CLIENT_BUILD_NUMBER, CLIENT_MASTERING_NUMBER};
use crate::error::{Error, Result};
use crate::http::{anonymous_client, icloud_headers};
use crate::session::SessionData;

/// One Hide My Email alias, parsed from the raw `v2/hme/list` JSON.
///
/// Fields beyond the core identity (`anonymous_id`, `hme`, `label`, `note`)
/// are optional because the upstream service occasionally omits them.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HmeAlias {
    pub anonymous_id: String,
    pub hme: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub forward_to_email: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    /// Unix milliseconds from the `createTimestamp` field (when present).
    #[serde(default)]
    pub create_timestamp: Option<i64>,
}

impl HmeAlias {
    fn from_entry(entry: &Value) -> Option<Self> {
        let anonymous_id = entry.get("anonymousId")?.as_str()?.to_string();
        let hme = entry.get("hme")?.as_str()?.to_string();
        Some(Self {
            anonymous_id,
            hme,
            label: entry
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            note: entry
                .get("note")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            forward_to_email: entry
                .get("forwardToEmail")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            is_active: entry.get("isActive").and_then(|v| v.as_bool()),
            origin: entry
                .get("origin")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            domain: entry
                .get("domain")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            create_timestamp: entry.get("createTimestamp").and_then(|v| v.as_i64()),
        })
    }

    /// Extract every alias entry from a `v2/hme/list` response.
    pub fn parse_list_response(value: &Value) -> Vec<Self> {
        value
            .get("result")
            .and_then(|r| r.get("hmeEmails"))
            .and_then(|e| e.as_array())
            .map(|arr| arr.iter().filter_map(Self::from_entry).collect())
            .unwrap_or_default()
    }
}

pub struct HideMyEmailClient {
    client: reqwest::Client,
    session: SessionData,
    base_url: String,
    host: String,
    client_id: String,
}

impl HideMyEmailClient {
    pub fn new(session: SessionData) -> Result<Self> {
        let url = session
            .webservices
            .as_ref()
            .and_then(|w| w.get("premiummailsettings"))
            .and_then(|x| x.get("url"))
            .and_then(|u| u.as_str())
            .ok_or_else(|| {
                Error::HideMyEmail(
                    "session missing premiummailsettings URL (iCloud+ / re-login required)".into(),
                )
            })?;
        let base = url.trim_end_matches('/').to_string();
        let host = url::Url::parse(&base)
            .map_err(|e| Error::HideMyEmail(e.to_string()))?
            .host_str()
            .unwrap_or("")
            .to_string();
        let client_id = session
            .client_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok(Self {
            client: anonymous_client()?,
            session,
            base_url: base,
            host,
            client_id,
        })
    }

    fn dsid(&self) -> Result<String> {
        self.session
            .dsid
            .clone()
            .ok_or_else(|| Error::HideMyEmail("session missing dsid (re-login)".into()))
    }

    fn common_query(&self) -> Result<Vec<(&'static str, String)>> {
        Ok(vec![
            ("clientBuildNumber", CLIENT_BUILD_NUMBER.to_string()),
            ("clientMasteringNumber", CLIENT_MASTERING_NUMBER.to_string()),
            ("clientId", self.client_id.clone()),
            ("dsid", self.dsid()?),
        ])
    }

    fn request_headers(&self) -> reqwest::header::HeaderMap {
        icloud_headers(&self.session, &self.host)
    }

    /// Convenience wrapper: call [`list_aliases`](Self::list_aliases) and
    /// parse the response into [`HmeAlias`] structs, sorted by label (case
    /// insensitive) then by email.
    pub async fn list_aliases_parsed(&self) -> Result<Vec<HmeAlias>> {
        let value = self.list_aliases().await?;
        let mut aliases = HmeAlias::parse_list_response(&value);
        aliases.sort_by(|a, b| {
            a.label
                .to_ascii_lowercase()
                .cmp(&b.label.to_ascii_lowercase())
                .then_with(|| a.hme.cmp(&b.hme))
        });
        Ok(aliases)
    }

    pub async fn list_aliases(&self) -> Result<Value> {
        let path = format!("{}/v2/hme/list", self.base_url);
        let resp = self
            .client
            .get(path)
            .query(&self.common_query()?)
            .headers(self.request_headers())
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::HideMyEmail(format!(
                "list HTTP {}: {}",
                status,
                crate::truncate(&body, 400)
            )));
        }
        serde_json::from_str(&body).map_err(Error::Json)
    }

    pub async fn generate(&self, lang_code: &str) -> Result<Option<String>> {
        let path = format!("{}/v1/hme/generate", self.base_url);
        let resp = self
            .client
            .post(path)
            .query(&self.common_query()?)
            .headers(self.request_headers())
            .json(&serde_json::json!({ "langCode": lang_code }))
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::HideMyEmail(format!(
                "generate HTTP {}: {}",
                status,
                crate::truncate(&body, 400)
            )));
        }
        let v: Value = serde_json::from_str(&body)?;
        Ok(v["result"]["hme"].as_str().map(|s| s.to_string()))
    }

    pub async fn reserve(&self, email: &str, label: &str, note: &str) -> Result<bool> {
        let path = format!("{}/v1/hme/reserve", self.base_url);
        #[derive(Serialize)]
        struct Body<'a> {
            hme: &'a str,
            label: &'a str,
            note: &'a str,
        }
        let resp = self
            .client
            .post(path)
            .query(&self.common_query()?)
            .headers(self.request_headers())
            .json(&Body {
                hme: email,
                label,
                note,
            })
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::HideMyEmail(format!(
                "reserve HTTP {}: {}",
                status,
                crate::truncate(&body, 400)
            )));
        }
        let v: Value = serde_json::from_str(&body)?;
        Ok(v["success"].as_bool().unwrap_or(false))
    }

    pub async fn deactivate(&self, anonymous_id: &str) -> Result<bool> {
        self.post_id_action("/v1/hme/deactivate", anonymous_id)
            .await
    }

    pub async fn delete_alias(&self, anonymous_id: &str) -> Result<bool> {
        self.post_id_action("/v1/hme/delete", anonymous_id).await
    }

    async fn post_id_action(&self, subpath: &str, anonymous_id: &str) -> Result<bool> {
        let path = format!("{}{}", self.base_url, subpath);
        let resp = self
            .client
            .post(path)
            .query(&self.common_query()?)
            .headers(self.request_headers())
            .json(&serde_json::json!({ "anonymousId": anonymous_id }))
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::HideMyEmail(format!(
                "{subpath} HTTP {}: {}",
                status,
                crate::truncate(&body, 400)
            )));
        }
        let v: Value = serde_json::from_str(&body)?;
        Ok(v["success"].as_bool().unwrap_or(false))
    }
}
