//! CloudKit web client for iCloud Reminders (`com.apple.reminders`).
//! Ported from [icloud-reminders-cli/internal/cloudkit](https://github.com/tarekbecker/icloud-reminders-cli).

use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use reqwest::Url;
use serde_json::{json, Map, Value};
use tokio::time::sleep;

use crate::error::{Error, Result};
use crate::http::{anonymous_client, icloud_headers};
use crate::session::SessionData;

const CK_MAX_ATTEMPTS: u32 = 6;

fn retry_delay(headers: &reqwest::header::HeaderMap, attempt: u32) -> Duration {
    if let Some(v) = headers.get(RETRY_AFTER).and_then(|h| h.to_str().ok()) {
        if let Ok(secs) = v.parse::<u64>() {
            return Duration::from_secs(secs.clamp(1, 120));
        }
    }
    let secs = 2_u64.saturating_pow(attempt.min(5)).clamp(1, 30);
    Duration::from_secs(secs)
}

pub struct CloudKitClient {
    client: reqwest::Client,
    ck_base: String,
    host: String,
    session: SessionData,
    container: String,
    zone: String,
}

impl CloudKitClient {
    pub fn new(session: SessionData, container: &str, zone: &str) -> Result<Self> {
        if session.ck_base_url.is_empty() {
            return Err(Error::Session("missing ck_base_url".into()));
        }
        let mut base = session.ck_base_url.clone();
        if !base.ends_with('/') {
            base.push('/');
        }
        let host = Url::parse(base.trim_end_matches('/'))
            .map_err(|e| Error::Session(e.to_string()))?
            .host_str()
            .unwrap_or("")
            .to_string();
        Ok(Self {
            client: anonymous_client()?,
            ck_base: base,
            host,
            session,
            container: container.to_string(),
            zone: zone.to_string(),
        })
    }

    pub fn reminders(session: SessionData) -> Result<Self> {
        Self::new(session, "com.apple.reminders", "Reminders")
    }

    pub fn notes(session: SessionData) -> Result<Self> {
        Self::new(session, "com.apple.notes", "Notes")
    }

    pub fn container(&self) -> &str {
        &self.container
    }

    pub fn zone(&self) -> &str {
        &self.zone
    }

    fn ck_url(&self, path: &str) -> String {
        format!("{}{}", self.ck_base, path.trim_start_matches('/'))
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let url = self.ck_url(path);
        let headers = icloud_headers(&self.session, &self.host);

        let mut last_body;
        let mut attempt = 0u32;

        loop {
            let resp = self
                .client
                .post(url.clone())
                .headers(headers.clone())
                .json(body)
                .send()
                .await?;

            let status = resp.status();
            let resp_headers = resp.headers().clone();
            let bytes = resp.bytes().await?;

            if status.is_success() {
                return serde_json::from_slice(&bytes).map_err(Error::Json);
            }

            last_body = String::from_utf8_lossy(&bytes).trim().to_string();
            let code = status.as_u16();
            if matches!(code, 429 | 503) && attempt + 1 < CK_MAX_ATTEMPTS {
                sleep(retry_delay(&resp_headers, attempt)).await;
                attempt += 1;
                continue;
            }

            return Err(Error::Api {
                status: code,
                body: crate::truncate(&last_body, 500).to_string(),
            });
        }
    }

    pub async fn get_owner_id(&self) -> Result<String> {
        let path = format!(
            "database/1/{}/production/private/zones/list",
            self.container
        );
        let result = self.post(&path, &json!({})).await?;
        if let Some(zones) = result["zones"].as_array() {
            for z in zones {
                let zone_id = &z["zoneID"];
                if zone_id["zoneName"].as_str() == Some(&self.zone) {
                    if let Some(owner) = zone_id["ownerRecordName"].as_str() {
                        return Ok(owner.to_string());
                    }
                }
            }
            if let Some(z) = zones.first() {
                if let Some(owner) = z["zoneID"]["ownerRecordName"].as_str() {
                    return Ok(owner.to_string());
                }
            }
        }
        Err(Error::Api {
            status: 404,
            body: format!("{} zone not found", self.zone),
        })
    }

    pub async fn changes_zone(
        &self,
        owner_id: &str,
        sync_token: Option<&str>,
        desired_keys: &[&str],
        desired_record_types: Option<&[&str]>,
    ) -> Result<Value> {
        let mut spec = Map::new();
        spec.insert(
            "zoneID".to_string(),
            json!({"zoneName": &self.zone, "ownerRecordName": owner_id}),
        );
        spec.insert("desiredKeys".to_string(), json!(desired_keys));
        if let Some(types) = desired_record_types {
            spec.insert("desiredRecordTypes".to_string(), json!(types));
        }
        if let Some(t) = sync_token {
            if !t.is_empty() {
                spec.insert("syncToken".to_string(), json!(t));
            }
        }
        let path = format!(
            "database/1/{}/production/private/changes/zone",
            self.container
        );
        self.post(&path, &json!({ "zones": [Value::Object(spec)] }))
            .await
    }

    pub async fn modify_records(&self, owner_id: &str, operations: Vec<Value>) -> Result<Value> {
        let path = format!(
            "database/1/{}/production/private/records/modify",
            self.container
        );
        let body = json!({
            "zoneID": {
                "zoneName": &self.zone,
                "ownerRecordName": owner_id,
            },
            "operations": operations,
            "atomic": true,
        });
        self.post(&path, &body).await
    }

    pub async fn lookup_records(
        &self,
        owner_id: &str,
        record_names: &[&str],
        desired_keys: &[&str],
    ) -> Result<Value> {
        let path = format!(
            "database/1/{}/production/private/records/lookup",
            self.container
        );
        let records: Vec<Value> = record_names
            .iter()
            .map(|rn| json!({ "recordName": rn }))
            .collect();
        let body = json!({
            "records": records,
            "zoneID": {
                "zoneName": &self.zone,
                "ownerRecordName": owner_id,
            },
            "desiredKeys": desired_keys,
        });
        self.post(&path, &body).await
    }

    /// Check CloudKit modify_records response for per-record server errors.
    pub fn check_record_errors(result: &Value, wrap: impl Fn(String) -> Error) -> Result<()> {
        if let Some(recs) = result["records"].as_array() {
            for r in recs {
                if let Some(code) = r["serverErrorCode"].as_str() {
                    if !code.is_empty() {
                        let reason = r["reason"].as_str().unwrap_or("");
                        return Err(wrap(format!("CloudKit {code}: {reason}")));
                    }
                }
            }
        }
        Ok(())
    }

    /// Drive a full CloudKit zone-changes loop and return all records across pages.
    ///
    /// `sync_token` is read and updated in-place so callers can persist it between runs.
    pub async fn sync_zone_changes(
        &self,
        sync_token: &mut Option<String>,
        desired_keys: &[&str],
        desired_record_types: Option<&[&str]>,
        owner_id: &str,
    ) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        loop {
            let token_str = sync_token.clone().unwrap_or_default();
            let token_ref = if token_str.is_empty() {
                None
            } else {
                Some(token_str.as_str())
            };
            let data = self
                .changes_zone(owner_id, token_ref, desired_keys, desired_record_types)
                .await?;
            let Some(zone_resp) = data["zones"].as_array().and_then(|z| z.first()) else {
                break;
            };
            let records = zone_resp["records"].as_array().cloned().unwrap_or_default();
            let more = zone_resp["moreComing"].as_bool().unwrap_or(false);
            if let Some(t) = zone_resp["syncToken"].as_str() {
                if !t.is_empty() {
                    *sync_token = Some(t.to_string());
                }
            }
            all.extend(records);
            if !more {
                break;
            }
        }
        Ok(all)
    }
}

// ── Shared CloudKit record field extraction ───────────────

/// Extract a string field value from CloudKit record fields.
pub fn ck_field_string(fields: &serde_json::Map<String, Value>, key: &str) -> String {
    fields
        .get(key)
        .and_then(|f| f.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Extract an integer field value from CloudKit record fields.
pub fn ck_field_int(fields: &serde_json::Map<String, Value>, key: &str) -> i32 {
    fields
        .get(key)
        .and_then(|f| f.get("value"))
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|x| x as i64)))
        .unwrap_or(0) as i32
}

/// Extract an optional i64 field value (returns None for 0 or missing).
pub fn ck_field_int64(fields: &serde_json::Map<String, Value>, key: &str) -> Option<i64> {
    let val = fields.get(key)?.get("value")?;
    let v = val.as_i64().or_else(|| val.as_f64().map(|x| x as i64))?;
    if v == 0 {
        None
    } else {
        Some(v)
    }
}

/// Extract a reference field's recordName.
pub fn ck_field_ref(fields: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    fields
        .get(key)?
        .get("value")?
        .get("recordName")?
        .as_str()
        .map(|s| s.to_string())
}

/// Extract a timestamp field (millis i64).
pub fn ck_field_timestamp(fields: &serde_json::Map<String, Value>, key: &str) -> Option<i64> {
    let val = fields.get(key)?.get("value")?;
    val.as_i64().or_else(|| val.as_f64().map(|f| f as i64))
}

/// Resolve owner-id from cache or fetch it once from CloudKit.
/// Pass `&mut cache.owner_id` (i.e. `&mut Option<String>`).
pub async fn ensure_owner_id(
    cache_owner: &mut Option<String>,
    ck: &CloudKitClient,
) -> crate::error::Result<String> {
    if let Some(o) = cache_owner.as_deref() {
        return Ok(o.to_string());
    }
    let id = ck.get_owner_id().await?;
    *cache_owner = Some(id.clone());
    Ok(id)
}

/// Base64-encode a UTF-8 string (for CloudKit BYTES fields like TitleEncrypted).
pub fn b64_encode_str(text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
}

/// Extract the first record's `recordChangeTag` from a modify_records response.
pub fn first_change_tag(result: &Value) -> Option<String> {
    result["records"]
        .as_array()
        .and_then(|recs| recs.first())
        .and_then(|r| r["recordChangeTag"].as_str())
        .map(|s| s.to_string())
}

impl CloudKitClient {
    /// Lightweight probe (same as Go `probeCloudKit`).
    pub async fn probe(&self) -> bool {
        let path = format!(
            "database/1/{}/production/private/zones/list",
            self.container
        );
        self.post(&path, &json!({})).await.is_ok()
    }
}
