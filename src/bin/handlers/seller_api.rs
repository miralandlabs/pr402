//! Canonical seller HTTP paths (v1.1) and helpers for path-param routes.

pub const SELLERS_PREFIX: &str = "/api/v1/facilitator/sellers/";
pub const PROVISION_TX: &str = "/api/v1/facilitator/sellers/provision-tx";
pub const PAYMENT_REQUIRED_ENRICH: &str = "/api/v1/facilitator/payment-required/enrich";

pub const PREVIEW_TEMPLATE: &str = "/api/v1/facilitator/sellers/{wallet}/preview";
pub const CHALLENGE_TEMPLATE: &str = "/api/v1/facilitator/sellers/{wallet}/challenge";
pub const REGISTER_TEMPLATE: &str = "/api/v1/facilitator/sellers/{wallet}/register";
pub const RETIRE_TEMPLATE: &str = "/api/v1/facilitator/sellers/{wallet}/retire";
pub const RAIL_TEMPLATE: &str = "/api/v1/facilitator/sellers/{wallet}/rails/{scheme}";

/// `/api/v1/facilitator/sellers/{wallet}/preview` | `.../challenge` | `.../register` | `.../retire`
pub fn parse_sellers_wallet_suffix(path: &str, suffix: &str) -> Option<String> {
    if !path.starts_with(SELLERS_PREFIX) || !path.ends_with(suffix) {
        return None;
    }
    let mid = &path[SELLERS_PREFIX.len()..path.len() - suffix.len()];
    if mid.is_empty() || mid.contains('/') {
        return None;
    }
    Some(mid.to_string())
}

/// `/api/v1/facilitator/sellers/{wallet}/rails/{scheme}`
pub fn parse_sellers_rail(path: &str) -> Option<(String, String)> {
    if !path.starts_with(SELLERS_PREFIX) {
        return None;
    }
    let rest = &path[SELLERS_PREFIX.len()..];
    let (wallet, scheme) = rest.split_once("/rails/")?;
    if wallet.is_empty() || scheme.is_empty() || scheme.contains('/') {
        return None;
    }
    Some((wallet.to_string(), scheme.to_string()))
}

/// Rebuild query string for handlers that expect `wallet` + `scheme` (+ optional `asset`).
pub fn discovery_query(wallet: &str, scheme: &str, asset: Option<&str>) -> String {
    match asset.filter(|s| !s.is_empty()) {
        Some(a) => format!("wallet={wallet}&scheme={scheme}&asset={a}"),
        None => format!("wallet={wallet}&scheme={scheme}"),
    }
}

pub fn preview_query(wallet: &str) -> String {
    format!("wallet={wallet}")
}

pub fn challenge_query(wallet: &str) -> String {
    preview_query(wallet)
}
