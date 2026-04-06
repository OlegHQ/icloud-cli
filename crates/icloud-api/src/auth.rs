//! Native iCloud SRP authentication and session bootstrap (Reminders CloudKit URL).
//! Flow matches [icloud-reminders-cli/internal/auth](https://github.com/tarekbecker/icloud-reminders-cli).

use base64::Engine;
use cookie_store::CookieStore as InnerCookieStore;
use reqwest::cookie::CookieStore as ReqwestCookieStore;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use reqwest_cookie_store::CookieStoreMutex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tracing::{debug, warn};

use crate::apple_srp::{SrpClient, SrpParams};
use crate::error::{Error, Result};
use crate::session::{SessionCookie, SessionData};

pub const AUTH_ENDPOINT: &str = "https://idmsa.apple.com/appleauth/auth";
pub const SETUP_ENDPOINT: &str = "https://setup.icloud.com/setup/ws/1";
pub const HOME_ENDPOINT: &str = "https://www.icloud.com";
pub const WIDGET_KEY: &str = "d39ba9916b7251055b22c7f910e2ea796ee65e98b2ddecea8f5dde8d9d1a815d";

pub const CLIENT_BUILD_NUMBER: &str = "2546Build34";
pub const CLIENT_MASTERING_NUMBER: &str = "2546Build34";

/// Info about a phone number available for SMS 2FA.
#[derive(Debug, Clone)]
pub struct TrustedPhone {
    pub id: u64,
    pub number: String,
}

/// 2FA state after SRP auth completes with 409.
#[derive(Debug, Clone)]
pub struct TwoFactorInfo {
    /// If true, the 2FA code was pushed to trusted Apple devices automatically.
    pub has_trusted_devices: bool,
    /// Phone numbers available for SMS-based 2FA (fallback).
    pub trusted_phones: Vec<TrustedPhone>,
    /// Expected code length (usually 6).
    pub code_length: u32,
}

pub struct AuthFlow {
    client: Client,
    cookie_jar: Arc<CookieStoreMutex>,
    username: String,
    password: String,
    client_id: String,
    auth_attr: String,
    session_id: String,
    scnt: String,
    auth_token: String,
    trust_token: String,
    data: SessionData,
}

impl AuthFlow {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        let frame = uuid::Uuid::new_v4().to_string().to_lowercase();
        let client_id = format!("auth-{frame}");
        let cookie_jar = Arc::new(CookieStoreMutex::new(InnerCookieStore::default()));
        let client = Client::builder()
            .cookie_provider(cookie_jar.clone())
            .build()
            .expect("reqwest client");
        Self {
            client,
            cookie_jar,
            username: username.into(),
            password: password.into(),
            client_id,
            auth_attr: String::new(),
            session_id: String::new(),
            scnt: String::new(),
            auth_token: String::new(),
            trust_token: String::new(),
            data: SessionData {
                ck_base_url: String::new(),
                session_token: None,
                trust_token: None,
                account_country: None,
                session_id: None,
                scnt: None,
                dsid: None,
                cookies: vec![],
                created_at: None,
                webservices: None,
                client_id: None,
                apple_id: None,
            },
        }
    }

    /// Convenience: full login in one call. Returns error if 2FA is needed but no code provided.
    pub async fn login_full(mut self, two_factor_code: Option<String>) -> Result<SessionData> {
        let tfa = self.login_srp().await?;
        if tfa.is_some() {
            let code = two_factor_code.ok_or_else(|| {
                Error::Auth(
                    "two-factor authentication required (pass --code or ICLOUD_2FA_CODE)".into(),
                )
            })?;
            self.submit_2fa(&code).await?;
        }
        self.finish_login().await
    }

    /// Perform SRP authentication. Returns `Some(TwoFactorInfo)` if 2FA is required.
    /// After this, call [`submit_2fa`] (or [`request_sms_code`] + [`submit_sms_code`]) then [`finish_login`].
    pub async fn login_srp(&mut self) -> Result<Option<TwoFactorInfo>> {
        self.auth_start().await?;
        self.device_key_challenge().await?;
        self.auth_federate().await?;
        let needs_2fa = self.srp_auth().await?;
        if needs_2fa {
            let info = self.auth_options().await?;
            Ok(Some(info))
        } else {
            Ok(None)
        }
    }

    /// Submit a trusted-device 2FA code.
    pub async fn submit_2fa(&mut self, code: &str) -> Result<()> {
        self.submit_two_factor(code).await
    }

    /// Request an SMS code to a specific phone (by ID from [`TwoFactorInfo::trusted_phones`]).
    pub async fn request_sms_code(&self, phone_id: u64) -> Result<()> {
        let body = json!({"phoneNumber": {"id": phone_id}, "mode": "sms"});
        let resp = self
            .client
            .put(format!("{AUTH_ENDPOINT}/verify/phone"))
            .headers(self.apple_auth_headers())
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let t = resp.text().await.unwrap_or_default();
            return Err(Error::Auth(format!("request SMS failed: {status} {t}")));
        }
        debug!("SMS code requested for phone id {phone_id}");
        Ok(())
    }

    /// Submit an SMS 2FA code (use after [`request_sms_code`]).
    pub async fn submit_sms_code(&mut self, code: &str, phone_id: u64) -> Result<()> {
        let body = json!({
            "securityCode": {"code": code.trim()},
            "phoneNumber": {"id": phone_id},
            "mode": "sms",
        });
        let resp = self
            .client
            .post(format!("{AUTH_ENDPOINT}/verify/phone/securitycode"))
            .headers(self.apple_auth_headers())
            .json(&body)
            .send()
            .await?;
        self.capture_session_headers(&resp);
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 204 {
            let t = resp.text().await.unwrap_or_default();
            return Err(Error::Auth(format!("SMS 2FA failed: {status} {t}")));
        }
        Ok(())
    }

    /// Complete login: get trust token, account login, return session data.
    pub async fn finish_login(mut self) -> Result<SessionData> {
        if let Err(e) = self.get_trust().await {
            warn!("getTrust failed (non-fatal): {e}");
        }
        self.account_login().await?;

        self.data.session_token = Some(self.auth_token.clone());
        self.data.trust_token = Some(self.trust_token.clone());
        self.data.session_id = Some(self.session_id.clone());
        self.data.scnt = Some(self.scnt.clone());
        self.data.cookies = self.extract_cookies_from_jar();
        self.data.created_at = Some(chrono::Utc::now().to_rfc3339());
        self.data.client_id = Some(self.client_id.clone());

        if self.data.ck_base_url.is_empty() {
            return Err(Error::Auth("missing ck_base_url after accountLogin".into()));
        }
        self.data.apple_id = Some(self.username.clone());
        Ok(self.data)
    }

    async fn auth_start(&mut self) -> Result<()> {
        let url = format!(
            "{AUTH_ENDPOINT}/authorize/signin?frame_id={}&language=en_US&skVersion=7&iframeId={}&client_id={}&redirect_uri=https://www.icloud.com&response_type=code&response_mode=web_message&state={}&authVersion=latest",
            self.client_id, self.client_id, WIDGET_KEY, self.client_id
        );
        let resp = self
            .client
            .get(url)
            .header("Accept", "*/*")
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36",
            )
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Auth(format!("authStart HTTP {}", resp.status())));
        }
        self.capture_session_headers(&resp);
        Ok(())
    }

    /// POST /verify/device/key/challenge — initiates device verification session.
    async fn device_key_challenge(&mut self) -> Result<()> {
        let body = json!({"passkeyAutofill": false});
        let resp = self
            .client
            .post(format!("{AUTH_ENDPOINT}/verify/device/key/challenge"))
            .headers(self.apple_auth_headers())
            .json(&body)
            .send()
            .await?;
        self.capture_session_headers(&resp);
        let status = resp.status();
        debug!("deviceKeyChallenge HTTP {status}");
        // Non-fatal: some accounts may not support this
        if !status.is_success() {
            debug!("deviceKeyChallenge failed (non-fatal)");
        }
        Ok(())
    }

    async fn auth_federate(&mut self) -> Result<()> {
        let body = json!({"accountName": self.username, "rememberMe": true});
        let resp = self
            .client
            .post(format!(
                "{AUTH_ENDPOINT}/federate?isRememberMeEnabled=true"
            ))
            .headers(self.apple_auth_headers())
            .json(&body)
            .send()
            .await?;
        self.capture_session_headers(&resp);
        let status = resp.status();
        if !status.is_success() {
            let t = resp.text().await.unwrap_or_default();
            return Err(Error::Auth(format!(
                "authFederate HTTP {}: {}",
                status,
                crate::truncate(&t, 500)
            )));
        }
        Ok(())
    }

    /// Returns `true` if 2FA is required (HTTP 409 on complete).
    async fn srp_auth(&mut self) -> Result<bool> {
        let params = SrpParams::apple_2048();
        let mut srp = SrpClient::new(params, None);
        let a_b64 = srp.a_b64();

        let init_body = json!({
            "a": a_b64,
            "accountName": self.username,
            "protocols": ["s2k", "s2k_fo"]
        });
        let resp = self
            .client
            .post(format!("{AUTH_ENDPOINT}/signin/init"))
            .headers(self.apple_auth_headers())
            .json(&init_body)
            .send()
            .await?;
        self.capture_session_headers(&resp);
        let status = resp.status();
        if !status.is_success() {
            let t = resp.text().await.unwrap_or_default();
            return Err(Error::Auth(format!(
                "authInit HTTP {}: {}",
                status,
                crate::truncate(&t, 500)
            )));
        }
        let init: Value = resp.json().await?;
        let iteration = init["iteration"].as_u64().unwrap_or(0) as u32;
        let salt = base64_decode(init["salt"].as_str().unwrap_or(""))?;
        let b = base64_decode(init["b"].as_str().unwrap_or(""))?;
        let c = init["c"].as_str().unwrap_or("").to_string();

        let pass_hash = Sha256::digest(self.password.as_bytes());
        let mut pass_key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha256>(pass_hash.as_slice(), &salt, iteration, &mut pass_key);

        srp.process_challenge(&self.username, &pass_key, &salt, &b);

        let m1 = srp.m1_b64();
        let m2 = srp.m2_b64();

        let mut complete = json!({
            "accountName": self.username,
            "rememberMe": true,
            "trustTokens": json!([]),
            "m1": m1,
            "c": c,
            "m2": m2,
        });
        if !self.trust_token.is_empty() {
            complete["trustTokens"] = json!([self.trust_token.clone()]);
        }

        let resp = self
            .client
            .post(format!(
                "{AUTH_ENDPOINT}/signin/complete?isRememberMeEnabled=true"
            ))
            .headers(self.apple_auth_headers())
            .json(&complete)
            .send()
            .await?;

        self.capture_session_headers(&resp);
        let status = resp.status();
        match status.as_u16() {
            200 => Ok(false),
            409 => Ok(true),
            401 | 403 => {
                let t = resp.text().await.unwrap_or_default();
                Err(Error::Auth(format!("invalid credentials: {status} {t}")))
            }
            _ => {
                let t = resp.text().await.unwrap_or_default();
                Err(Error::Auth(format!(
                    "authComplete HTTP {}: {}",
                    status,
                    crate::truncate(&t, 500)
                )))
            }
        }
    }

    async fn submit_two_factor(&mut self, code: &str) -> Result<()> {
        let body = json!({"securityCode": {"code": code.trim()}});
        let resp = self
            .client
            .post(format!(
                "{AUTH_ENDPOINT}/verify/trusteddevice/securitycode"
            ))
            .headers(self.apple_auth_headers())
            .json(&body)
            .send()
            .await?;
        self.capture_session_headers(&resp);
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 204 {
            let t = resp.text().await.unwrap_or_default();
            return Err(Error::Auth(format!("2FA failed: {status} {t}")));
        }
        Ok(())
    }

    /// GET /appleauth/auth — fetch auth options (trusted devices, phone numbers, code length).
    async fn auth_options(&mut self) -> Result<TwoFactorInfo> {
        let resp = self
            .client
            .get(AUTH_ENDPOINT)
            .headers(self.apple_auth_headers())
            .send()
            .await?;
        self.capture_session_headers(&resp);
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        debug!("authOptions HTTP {status}: {}", crate::truncate(&text, 500));

        let v: Value = serde_json::from_str(&text).unwrap_or_default();

        // Apple nests phone info under "phoneNumberVerification" or at the top level
        let pnv = if v.get("phoneNumberVerification").is_some() {
            &v["phoneNumberVerification"]
        } else {
            &v
        };

        let no_trusted = v["noTrustedDevices"].as_bool().unwrap_or(true);
        let code_length = pnv["securityCode"]["length"]
            .as_u64()
            .unwrap_or(6) as u32;
        let phones: Vec<TrustedPhone> = pnv["trustedPhoneNumbers"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        Some(TrustedPhone {
                            id: p["id"].as_u64()?,
                            number: p["numberWithDialCode"]
                                .as_str()
                                .or_else(|| p["obfuscatedNumber"].as_str())
                                .unwrap_or("???")
                                .to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(TwoFactorInfo {
            has_trusted_devices: !no_trusted,
            trusted_phones: phones,
            code_length,
        })
    }

    async fn get_trust(&mut self) -> Result<()> {
        let resp = self
            .client
            .get(format!("{AUTH_ENDPOINT}/2sv/trust"))
            .headers(self.apple_auth_headers())
            .send()
            .await?;
        if resp.status().as_u16() != 204 {
            return Err(Error::Auth(format!("getTrust HTTP {}", resp.status())));
        }
        if let Some(v) = resp.headers().get("x-apple-session-token") {
            self.auth_token = v.to_str().unwrap_or("").to_string();
        }
        if let Some(v) = resp.headers().get("x-apple-twosv-trust-token") {
            self.trust_token = v.to_str().unwrap_or("").to_string();
        }
        Ok(())
    }

    async fn account_login(&mut self) -> Result<()> {
        let token = if !self.auth_token.is_empty() {
            self.auth_token.clone()
        } else {
            self.session_token_from_jar()
        };
        if token.is_empty() {
            return Err(Error::Auth("no session token for accountLogin".into()));
        }

        let mut body = json!({
            "dsWebAuthToken": token,
            "extended_login": true,
        });
        if !self.trust_token.is_empty() {
            body["trustToken"] = json!(self.trust_token.clone());
        }
        let client_id = self.client_id.clone();
        let resp = self
            .client
            .post(format!("{SETUP_ENDPOINT}/accountLogin"))
            .query(&[
                ("requestId", uuid::Uuid::new_v4().to_string()),
                ("clientBuildNumber", CLIENT_BUILD_NUMBER.to_string()),
                ("clientMasteringNumber", CLIENT_MASTERING_NUMBER.to_string()),
                ("clientId", client_id),
            ])
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Origin", HOME_ENDPOINT)
            .header("Referer", format!("{HOME_ENDPOINT}/"))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Auth(format!(
                "accountLogin HTTP {}: {}",
                status,
                crate::truncate(&text, 500)
            )));
        }
        let v: Value = serde_json::from_str(&text)?;
        if let Some(dsid) = v["dsInfo"]["dsid"].as_str() {
            self.data.dsid = Some(dsid.to_string());
        }
        if let Some(ws) = v.get("webservices") {
            self.data.webservices = Some(ws.clone());
        }
        let ck = v["webservices"]["ckdatabasews"]["url"]
            .as_str()
            .ok_or_else(|| Error::Auth("no ckdatabasews.url".into()))?;
        self.data.ck_base_url = ck.to_string();
        debug!("obtained ck_base_url");
        Ok(())
    }

    fn apple_auth_headers(&self) -> HeaderMap {
        let mut h = HeaderMap::new();
        if !self.scnt.is_empty() {
            if let Ok(v) = HeaderValue::from_str(&self.scnt) {
                let _ = h.insert("scnt", v);
            }
        }
        if !self.session_id.is_empty() {
            if let Ok(v) = HeaderValue::from_str(&self.session_id) {
                let _ = h.insert("x-apple-id-session-id", v);
            }
        }
        h.insert("x-requested-with", HeaderValue::from_static("XMLHttpRequest"));
        h.insert("content-type", HeaderValue::from_static("application/json"));
        h.insert("accept", HeaderValue::from_static("application/json"));
        h.insert("referer", HeaderValue::from_static("https://idmsa.apple.com/"));
        h.insert("origin", HeaderValue::from_static("https://idmsa.apple.com"));
        h.insert(
            "x-apple-widget-key",
            HeaderValue::from_static(WIDGET_KEY),
        );
        h.insert("x-apple-i-require-ue", HeaderValue::from_static("true"));
        if !self.auth_attr.is_empty() {
            if let Ok(v) = HeaderValue::from_str(&self.auth_attr) {
                let _ = h.insert("x-apple-auth-attributes", v);
            }
        }
        h.insert(
            "user-agent",
            HeaderValue::from_static(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36",
            ),
        );
        h.insert("x-apple-mandate-security-upgrade", HeaderValue::from_static("0"));
        h.insert("x-apple-oauth-client-id", HeaderValue::from_static(WIDGET_KEY));
        h.insert(
            "x-apple-oauth-client-type",
            HeaderValue::from_static("firstPartyAuth"),
        );
        h.insert(
            "x-apple-oauth-redirect-uri",
            HeaderValue::from_static("https://www.icloud.com"),
        );
        h.insert(
            "x-apple-oauth-require-grant-code",
            HeaderValue::from_static("true"),
        );
        h.insert(
            "x-apple-oauth-response-mode",
            HeaderValue::from_static("web_message"),
        );
        h.insert(
            "x-apple-oauth-response-type",
            HeaderValue::from_static("code"),
        );
        if let Ok(v) = HeaderValue::from_str(&self.client_id) {
            let _ = h.insert("x-apple-oauth-state", v.clone());
            let _ = h.insert("x-apple-frame-id", v);
        }
        h.insert("x-apple-offer-security-upgrade", HeaderValue::from_static("1"));
        h.insert(
            reqwest::header::HeaderName::from_static("x-apple-i-fd-client-info"),
            HeaderValue::from_static(
                r#"{"U":"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.3.1 Safari/605.1.15","L":"en-US","Z":"GMT+00:00","V":"1.1","F":""}"#,
            ),
        );
        h
    }

    fn capture_session_headers(&mut self, resp: &reqwest::Response) {
        if let Some(v) = resp.headers().get("x-apple-id-session-id") {
            self.session_id = v.to_str().unwrap_or("").to_string();
        }
        if let Some(v) = resp.headers().get("scnt") {
            self.scnt = v.to_str().unwrap_or("").to_string();
        }
        if let Some(v) = resp.headers().get("x-apple-auth-attributes") {
            self.auth_attr = v.to_str().unwrap_or("").to_string();
        }
    }

    fn extract_cookies_from_jar(&self) -> Vec<SessionCookie> {
        let Ok(guard) = self.cookie_jar.lock() else {
            return vec![];
        };
        let mut out = Vec::new();
        for c in guard.iter_any() {
            out.push(SessionCookie {
                name: c.name().to_string(),
                value: c.value().to_string(),
                domain: c.domain().map(|d| d.to_string()).unwrap_or_default(),
                path: c.path().unwrap_or("/").to_string(),
                expires: 0,
                secure: c.secure().unwrap_or(false),
            });
        }
        out
    }

    fn session_token_from_jar(&self) -> String {
        let Ok(idmsa) = "https://idmsa.apple.com/".parse::<reqwest::Url>() else {
            return String::new();
        };
        if let Some(h) = ReqwestCookieStore::cookies(self.cookie_jar.as_ref(), &idmsa) {
            if let Ok(s) = h.to_str() {
                for part in s.split(';') {
                    let part = part.trim();
                    if let Some(rest) = part.strip_prefix("X-APPLE-DS-WEB-SESSION-TOKEN=") {
                        return crate::session::unquote_cookie_value(rest);
                    }
                }
            }
        }
        String::new()
    }
}

impl AuthFlow {
    pub async fn validate_session(session: &SessionData) -> Result<bool> {
        let client = crate::http::anonymous_client()?;
        let client_id = session
            .client_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let headers = crate::http::icloud_headers(session, "setup.icloud.com");
        let resp = client
            .post(format!("{SETUP_ENDPOINT}/validate"))
            .query(&[
                ("clientBuildNumber", CLIENT_BUILD_NUMBER.to_string()),
                ("clientMasteringNumber", CLIENT_MASTERING_NUMBER.to_string()),
                ("clientId", client_id),
            ])
            .headers(headers)
            .body("null")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(false);
        }
        let v: Value = resp.json().await?;
        Ok(v.get("dsInfo").is_some())
    }
}

fn base64_decode(s: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| Error::Auth(format!("base64: {e}")))
}

