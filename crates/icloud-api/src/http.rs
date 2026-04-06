//! Shared HTTP helpers: attach cookies from a persisted [`SessionData`](crate::session::SessionData).

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;

use crate::error::{Error, Result};
use crate::session::{unquote_cookie_value, SessionData};

/// Build a [`Client`] without an internal cookie jar — callers pass `Cookie` per request via
/// [`cookie_header`](Self::cookie_header).
pub fn anonymous_client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(120))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .map_err(Error::Http)
}

/// Combined `Cookie` header value for a request URL host (e.g. `pNN-ckdatabasews.icloud.com`).
pub fn cookie_header_for_host(session: &SessionData, host: &str) -> String {
    session
        .cookies
        .iter()
        .filter(|c| host_matches(&c.domain, host))
        .map(|c| format!("{}={}", c.name, unquote_cookie_value(&c.value)))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn insert_cookie_header(headers: &mut HeaderMap, session: &SessionData, host: &str) {
    let v = cookie_header_for_host(session, host);
    if !v.is_empty() {
        if let Ok(h) = HeaderValue::from_str(&v) {
            headers.insert(reqwest::header::COOKIE, h);
        }
    }
}

/// Standard iCloud API headers: cookies + JSON content-type/accept + origin/referer.
/// Callers needing different Content-Type or Accept can override after calling this.
pub fn icloud_headers(session: &SessionData, host: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    insert_cookie_header(&mut h, session, host);
    h.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h.insert(
        reqwest::header::ACCEPT,
        HeaderValue::from_static("application/json"),
    );
    h.insert(
        reqwest::header::ORIGIN,
        HeaderValue::from_static("https://www.icloud.com"),
    );
    h.insert(
        reqwest::header::REFERER,
        HeaderValue::from_static("https://www.icloud.com/"),
    );
    h
}

fn host_matches(cookie_domain: &str, request_host: &str) -> bool {
    let cd = cookie_domain.trim_start_matches('.');
    request_host.eq_ignore_ascii_case(cd)
        || (request_host.len() > cd.len() + 1
            && request_host
                .as_bytes()
                .get(request_host.len() - cd.len() - 1)
                == Some(&b'.')
            && request_host[request_host.len() - cd.len()..].eq_ignore_ascii_case(cd))
}
