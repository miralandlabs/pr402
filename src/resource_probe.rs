//! Unpaid HTTP probe: expect 402 with parseable PaymentRequired JSON.

use serde_json::Value;
use std::net::{IpAddr, SocketAddr};
#[cfg(test)]
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use tracing::warn;
use url::Url;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_BODY_BYTES: usize = 64 * 1024;

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 168)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || a >= 224)
        }
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let first = ip.segments()[0];
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0
                || (first == 0x2001 && ip.segments()[1] == 0x0db8))
        }
    }
}

async fn resolve_public_target(url: &str) -> Result<(Url, Option<(String, SocketAddr)>), String> {
    let parsed = Url::parse(url).map_err(|e| format!("invalid resource URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("resource URL scheme must be http or https".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("resource URL must not contain credentials".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "resource URL must include a host".to_string())?
        .to_string();
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "resource URL has no usable port".to_string())?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err(format!("resource URL resolves to non-public address {ip}"));
        }
        return Ok((parsed, None));
    }

    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("resource host DNS lookup failed: {e}"))?
        .collect();
    if addresses.is_empty() {
        return Err("resource host DNS lookup returned no addresses".into());
    }
    if let Some(address) = addresses.iter().find(|address| !is_public_ip(address.ip())) {
        return Err(format!(
            "resource host resolves to non-public address {}",
            address.ip()
        ));
    }

    Ok((parsed, Some((host, addresses[0]))))
}

/// Compare two URLs on scheme + host + port + path only, ignoring the query string.
///
/// The probed URL often carries example query args (so the endpoint reaches its 402
/// gate instead of a 400 input-validation error), while a seller's 402 may advertise
/// a canonical `resource.url` without those request-specific params. Binding on
/// origin+path keeps the liveness check meaningful without forcing query equality.
fn same_origin_path(a: &str, b: &str) -> bool {
    match (Url::parse(a), Url::parse(b)) {
        (Ok(ua), Ok(ub)) => {
            ua.scheme() == ub.scheme()
                && ua.host_str() == ub.host_str()
                && ua.port_or_known_default() == ub.port_or_known_default()
                && ua.path() == ub.path()
        }
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct ResourceProbeResult {
    pub ok: bool,
    pub http_status: Option<u16>,
    pub scheme: Option<String>,
    pub error: Option<String>,
}

pub async fn probe_resource_url(method: &str, url: &str) -> ResourceProbeResult {
    let (target, dns_override) = match resolve_public_target(url).await {
        Ok(target) => target,
        Err(error) => {
            return ResourceProbeResult {
                ok: false,
                http_status: None,
                scheme: None,
                error: Some(error),
            };
        }
    };

    let mut client_builder = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if let Some((host, address)) = dns_override {
        client_builder = client_builder.resolve(&host, address);
    }
    let client = match client_builder.build() {
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
        "GET" => client.get(target.clone()),
        "POST" => client.post(target.clone()),
        "PUT" => client.put(target.clone()),
        "PATCH" => client.patch(target.clone()),
        "DELETE" => client.delete(target.clone()),
        _ => {
            return ResourceProbeResult {
                ok: false,
                http_status: None,
                scheme: None,
                error: Some(format!("unsupported method {method_upper}")),
            };
        }
    };

    let mut resp = match req.send().await {
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

    let mut body = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) if body.len() + chunk.len() <= MAX_PROBE_BODY_BYTES => {
                body.extend_from_slice(&chunk);
            }
            Ok(Some(_)) => {
                return ResourceProbeResult {
                    ok: false,
                    http_status: Some(402),
                    scheme: None,
                    error: Some(format!(
                        "402 body exceeds {MAX_PROBE_BODY_BYTES} byte probe limit"
                    )),
                };
            }
            Ok(None) => break,
            Err(e) => {
                return ResourceProbeResult {
                    ok: false,
                    http_status: Some(402),
                    scheme: None,
                    error: Some(format!("402 body read: {e}")),
                };
            }
        }
    }
    let body_text = match String::from_utf8(body) {
        Ok(text) => text,
        Err(e) => {
            return ResourceProbeResult {
                ok: false,
                http_status: Some(402),
                scheme: None,
                error: Some(format!("402 body is not UTF-8: {e}")),
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
    if !resource_url
        .map(|ru| same_origin_path(ru, url))
        .unwrap_or(false)
    {
        return ResourceProbeResult {
            ok: false,
            http_status: Some(402),
            scheme: scheme.clone(),
            error: Some(format!(
                "402 resource.url origin/path mismatch (probed {url}, got {:?})",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_private_and_special_ip_ranges() {
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            "fd00::1".parse().unwrap(),
        ] {
            assert!(!is_public_ip(ip), "{ip} must not be probed");
        }
    }

    #[test]
    fn permits_public_ip_ranges() {
        assert!(is_public_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[tokio::test]
    async fn rejects_literal_private_target_before_http() {
        let error = resolve_public_target("http://127.0.0.1/admin")
            .await
            .unwrap_err();
        assert!(error.contains("non-public"));
    }
}
