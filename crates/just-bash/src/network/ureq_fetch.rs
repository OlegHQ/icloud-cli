//! Blocking HTTP via [`ureq`], wrapped for async [`FetchFn`](crate::commands::types::FetchFn) callers.

use crate::commands::types::{FetchFn, FetchResponse};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Build a [`FetchFn`] using `ureq` on [`tokio::task::spawn_blocking`].
///
/// This performs no allow-list checks. For `curl` in a sandbox, wrap the result with
/// [`crate::network::create_secure_fetch_fn`].
pub fn ureq_fetch_fn() -> FetchFn {
    Arc::new(|url, method, headers, body| {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || ureq_request(url, method, headers, body))
                .await
                .map_err(|e| format!("fetch join error: {e}"))?
        })
    })
}

fn ureq_request(
    url: String,
    method: String,
    headers: HashMap<String, String>,
    body: Option<String>,
) -> Result<FetchResponse, String> {
    let m = method.to_uppercase();
    let mut req = match m.as_str() {
        "GET" => ureq::get(&url),
        "HEAD" => ureq::head(&url),
        "POST" => ureq::post(&url),
        "PUT" => ureq::put(&url),
        "PATCH" => ureq::patch(&url),
        "DELETE" => ureq::delete(&url),
        other => ureq::request(other, &url),
    };

    req = req.timeout(Duration::from_secs(120));

    for (name, value) in headers {
        if !name.is_empty() {
            req = req.set(&name, &value);
        }
    }

    let resp = if let Some(ref b) = body {
        req.send_string(b)
    } else {
        req.call()
    }
    .map_err(|e| e.to_string())?;

    let status = resp.status();
    let final_url = resp.get_url().to_string();

    let mut out_headers = HashMap::new();
    for name in resp.headers_names() {
        if let Some(val) = resp.header(&name) {
            out_headers.insert(name, val.to_string());
        }
    }

    let body = resp.into_string().map_err(|e| e.to_string())?;

    Ok(FetchResponse {
        status,
        headers: out_headers,
        body,
        url: final_url,
    })
}
