//! Unpaid HTTP probe: expect 402 with parseable PaymentRequired JSON.

use serde_json::Value;
use std::time::Duration;
use tracing::warn;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct ResourceProbeResult {
    pub ok: bool,
    pub http_status: Option<u16>,
    pub scheme: Option<String>,
    pub error: Option<String>,
}

pub async fn probe_resource_url(method: &str, url: &str) -> ResourceProbeResult {
    let client = match reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ResourceProbeResult {
                ok: false,
                http_status: None,
                scheme: None,
                error: Some(format!("probe client: {e}")),
            };
        }
    };

    let method_upper = method.to_uppercase();
    let req = match method_upper.as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "PATCH" => client.patch(url),
        "DELETE" => client.delete(url),
        _ => {
            return ResourceProbeResult {
                ok: false,
                http_status: None,
                scheme: None,
                error: Some(format!("unsupported method {method_upper}")),
            };
        }
    };

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return ResourceProbeResult {
                ok: false,
                http_status: None,
                scheme: None,
                error: Some(format!("transport: {e}")),
            };
        }
    };

    let status = resp.status().as_u16();
    if status != 402 {
        return ResourceProbeResult {
            ok: false,
            http_status: Some(status),
            scheme: None,
            error: Some(format!("expected HTTP 402, got {status}")),
        };
    }

    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return ResourceProbeResult {
                ok: false,
                http_status: Some(402),
                scheme: None,
                error: Some(format!("402 body read: {e}")),
            };
        }
    };

    let parsed: Value = match serde_json::from_str(body_text.trim()) {
        Ok(v) => v,
        Err(e) => {
            return ResourceProbeResult {
                ok: false,
                http_status: Some(402),
                scheme: None,
                error: Some(format!("402 body not JSON: {e}")),
            };
        }
    };

    let scheme = parsed
        .get("accepts")
        .and_then(|a| a.as_array())
        .and_then(|arr| arr.first())
        .and_then(|line| line.get("scheme"))
        .and_then(|s| s.as_str())
        .map(str::to_string);

    if scheme.is_none() {
        warn!(target: "server_log", url = %url, "402 probe: accepts[0].scheme missing");
        return ResourceProbeResult {
            ok: false,
            http_status: Some(402),
            scheme: None,
            error: Some("402 JSON missing accepts[0].scheme".into()),
        };
    }

    let resource_url = parsed
        .get("resource")
        .and_then(|r| r.get("url"))
        .and_then(|u| u.as_str());
    if resource_url != Some(url) {
        return ResourceProbeResult {
            ok: false,
            http_status: Some(402),
            scheme: scheme.clone(),
            error: Some(format!(
                "402 resource.url mismatch (expected {url}, got {:?})",
                resource_url
            )),
        };
    }

    ResourceProbeResult {
        ok: true,
        http_status: Some(402),
        scheme,
        error: None,
    }
}
