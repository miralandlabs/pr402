//! Validation helpers for Layer 3 payable resource registration.

use url::Url;

/// Payload fields validated before persisting a `payable_resources` row.
#[derive(Debug, Clone)]
pub struct PayableResourceFields<'a> {
    pub resource_url: &'a str,
    pub http_method: &'a str,
    pub seller_resource_id: Option<&'a str>,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub use_case: Option<&'a str>,
    pub category: Option<&'a str>,
    pub tags: Option<&'a [String]>,
    pub scheme: &'a str,
    pub network: Option<&'a str>,
    pub intent_contract_url: Option<&'a str>,
}

pub fn validate_payable_resource(f: &PayableResourceFields<'_>) -> Result<(), String> {
    if f.resource_url.len() > 2048 {
        return Err("resource.resourceUrl exceeds 2048 chars".into());
    }
    if !(f.resource_url.starts_with("https://") || f.resource_url.starts_with("http://")) {
        return Err("resource.resourceUrl must start with https:// or http://".into());
    }
    let method = f.http_method.to_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return Err("resource.httpMethod must be GET, POST, PUT, PATCH, or DELETE".into());
    }
    if let Some(id) = f.seller_resource_id {
        if id.is_empty() || id.len() > 64 {
            return Err("resource.sellerResourceId must be 1..=64 characters".into());
        }
    }
    if f.title.chars().count() > 64 {
        return Err("resource.title exceeds 64 characters".into());
    }
    if f.title.trim().is_empty() {
        return Err("resource.title is required".into());
    }
    if let Some(d) = f.description {
        if d.chars().count() > 280 {
            return Err("resource.description exceeds 280 characters".into());
        }
    }
    if let Some(u) = f.use_case {
        if u.chars().count() > 255 {
            return Err("resource.useCase exceeds 255 characters".into());
        }
    }
    if let Some(c) = f.category {
        if c.len() > 32 {
            return Err("resource.category exceeds 32 characters".into());
        }
        if !c
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        {
            return Err("resource.category: lowercase ASCII letters, digits, and '-' only".into());
        }
    }
    if let Some(tags) = f.tags {
        if tags.len() > 5 {
            return Err("resource.tags accepts at most 5 entries".into());
        }
        for t in tags {
            if t.is_empty() || t.len() > 32 {
                return Err("resource.tags: each tag must be 1..=32 characters".into());
            }
            if !t
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
            {
                return Err("resource.tags: lowercase ASCII letters, digits, and '-' only".into());
            }
        }
    }
    if f.scheme != "exact" && f.scheme != "sla-escrow" {
        return Err("resource.scheme must be exact or sla-escrow".into());
    }
    if let Some(n) = f.network {
        if n.len() > 128 {
            return Err("resource.network exceeds 128 characters".into());
        }
    }
    if let Some(u) = f.intent_contract_url {
        if u.len() > 2048 {
            return Err("resource.intentContractUrl exceeds 2048 chars".into());
        }
        if !(u.starts_with("https://") || u.starts_with("http://")) {
            return Err("resource.intentContractUrl must start with https:// or http://".into());
        }
    }
    Ok(())
}

/// Host part of a URL (lowercase, no port normalization beyond url crate).
pub fn url_host(url_str: &str) -> Option<String> {
    Url::parse(url_str)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
}

/// Layer 3 `resource_url` host must match merchant `service_url` host.
pub fn resource_url_host_matches_service_url(resource_url: &str, service_url: &str) -> bool {
    match (url_host(resource_url), url_host(service_url)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields<'a>(
        resource_url: &'a str,
        title: &'a str,
        scheme: &'a str,
    ) -> PayableResourceFields<'a> {
        PayableResourceFields {
            resource_url,
            http_method: "GET",
            seller_resource_id: None,
            title,
            description: None,
            use_case: None,
            category: None,
            tags: None,
            scheme,
            network: None,
            intent_contract_url: None,
        }
    }

    #[test]
    fn validate_ok() {
        assert!(validate_payable_resource(&fields(
            "https://example.com/api/premium",
            "Premium API",
            "exact"
        ))
        .is_ok());
    }

    #[test]
    fn host_match() {
        assert!(resource_url_host_matches_service_url(
            "https://spl-token.hashspace.me/api/v1/buy",
            "https://spl-token.hashspace.me"
        ));
        assert!(!resource_url_host_matches_service_url(
            "https://evil.example/api",
            "https://spl-token.hashspace.me"
        ));
    }
}
