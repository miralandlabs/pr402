use super::*;

use pr402::db::{DbError, OwnerResourceEntry};
use pr402::payable_resource::{validate_payable_resource, PayableResourceFields};

pub const RESOURCES_PREFIX: &str = "/api/v1/facilitator/resources";
pub const REGISTER_CHALLENGE: &str = "/api/v1/facilitator/resources/register/challenge";
pub const REGISTER: &str = "/api/v1/facilitator/resources/register";
pub const RETIRE: &str = "/api/v1/facilitator/resources/retire";
pub const PROBE: &str = "/api/v1/facilitator/resources/probe";
pub const SELLERS_RESOURCES_SUFFIX: &str = "/resources";

const RESOURCE_DIRECTORY_DISCLAIMER: &str =
    "Advisory metadata only. Authoritative payment terms come from live HTTP 402 on each resourceUrl.";

pub async fn handle_resource_register_challenge(query: &str) -> Response<Body> {
    let wallet = query_param(query, "wallet");
    if wallet.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Missing wallet parameter");
    }
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Config error: {}", e),
            );
        }
    };
    let Some(secret) = pr402::parameters::resolve_onboard_hmac_secret(pr402_db()).await else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_ONBOARD_HMAC_SECRET not set (env or parameters table)",
        );
    };
    let ttl = pr402::parameters::resolve_onboard_challenge_ttl_sec(
        pr402_db(),
        cfg.onboard_challenge_ttl_sec,
    )
    .await
    .clamp(1, 3600);
    let (message, expires) =
        match pr402::onboard_auth::build_signed_onboard_message(secret.as_bytes(), &wallet, ttl) {
            Ok(x) => x,
            Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
        };
    let body = serde_json::json!({
        "wallet": wallet,
        "message": message,
        "expiresUnix": expires,
        "ttlSeconds": ttl,
    });
    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(body.to_string()))
        .unwrap()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourcePayload {
    resource_url: String,
    #[serde(default = "default_http_method")]
    http_method: String,
    #[serde(default)]
    seller_resource_id: Option<String>,
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    use_case: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    scheme: String,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    intent_contract_url: Option<String>,
    #[serde(default)]
    listing_opt_in: Option<bool>,
}

fn default_http_method() -> String {
    "GET".into()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceRegisterBody {
    wallet: String,
    message: String,
    signature: String,
    resource: ResourcePayload,
    #[serde(default)]
    source: Option<String>,
}

pub async fn handle_resource_register(body: Body) -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Resource registration requires DATABASE_URL.",
        );
    };
    let Some(secret) = pr402::parameters::resolve_onboard_hmac_secret(pr402_db()).await else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_ONBOARD_HMAC_SECRET not set",
        );
    };

    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let parsed: ResourceRegisterBody = match serde_json::from_str(&body_str) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };

    if let Err(e) = pr402::onboard_auth::verify_onboard_submission(
        secret.as_bytes(),
        &parsed.wallet,
        &parsed.message,
        &parsed.signature,
    ) {
        return error_response(StatusCode::UNAUTHORIZED, &e);
    }

    match db.resource_provider_verified(&parsed.wallet).await {
        Ok(true) => {}
        Ok(false) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "Complete merchant onboarding (Layer 2) before registering payable resources.",
            );
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("registry lookup failed: {}", e),
            );
        }
    }

    let r = &parsed.resource;
    let http_method = r.http_method.to_uppercase();
    let fields = PayableResourceFields {
        resource_url: &r.resource_url,
        http_method: &http_method,
        seller_resource_id: r.seller_resource_id.as_deref(),
        title: &r.title,
        description: r.description.as_deref(),
        use_case: r.use_case.as_deref(),
        category: r.category.as_deref(),
        tags: r.tags.as_deref(),
        scheme: &r.scheme,
        network: r.network.as_deref(),
        intent_contract_url: r.intent_contract_url.as_deref(),
    };
    if let Err(e) = validate_payable_resource(&fields) {
        return error_response(StatusCode::BAD_REQUEST, &e);
    }

    let service_url = match db.get_merchant_service_url(&parsed.wallet).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "Set discovery.serviceUrl on merchant register before listing resources (origin binding).",
            );
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("service_url lookup failed: {}", e),
            );
        }
    };

    if !pr402::payable_resource::resource_url_host_matches_service_url(
        &r.resource_url,
        &service_url,
    ) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "resource.resourceUrl host must match merchant serviceUrl host",
        );
    }

    let listing = r.listing_opt_in.unwrap_or(false);
    let source = parsed.source.as_deref().unwrap_or("register_ui");
    let source = if source == "register_api" {
        "register_api"
    } else {
        "register_ui"
    };

    match db
        .upsert_payable_resource(
            &parsed.wallet,
            &r.resource_url,
            &http_method,
            r.seller_resource_id.as_deref(),
            &r.title,
            r.description.as_deref(),
            r.use_case.as_deref(),
            r.category.as_deref(),
            r.tags.as_deref(),
            &r.scheme,
            r.network.as_deref(),
            r.intent_contract_url.as_deref(),
            None,
            listing,
            source,
        )
        .await
    {
        Ok(id) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(
                serde_json::json!({
                    "id": id,
                    "resourceUrl": r.resource_url,
                    "listingOptIn": listing,
                    "note": "Run a 402 probe before the resource appears in public search."
                })
                .to_string(),
            ))
            .unwrap(),
        Err(DbError::Query(m)) => error_response(StatusCode::BAD_REQUEST, &m),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("register failed: {}", e),
        ),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceRetireBody {
    wallet: String,
    message: String,
    signature: String,
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    resource_url: Option<String>,
}

pub async fn handle_resource_retire(body: Body) -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Resource retirement requires DATABASE_URL.",
        );
    };
    let Some(secret) = pr402::parameters::resolve_onboard_hmac_secret(pr402_db()).await else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_ONBOARD_HMAC_SECRET not set",
        );
    };

    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let parsed: ResourceRetireBody = match serde_json::from_str(&body_str) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };

    if parsed.id.is_none() && parsed.resource_url.as_deref().unwrap_or("").is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Provide id or resourceUrl");
    }

    if let Err(e) = pr402::onboard_auth::verify_onboard_submission(
        secret.as_bytes(),
        &parsed.wallet,
        &parsed.message,
        &parsed.signature,
    ) {
        return error_response(StatusCode::UNAUTHORIZED, &e);
    }

    match db
        .retire_payable_resource(&parsed.wallet, parsed.id, parsed.resource_url.as_deref())
        .await
    {
        Ok(n) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(
                serde_json::json!({ "retiredRowCount": n }).to_string(),
            ))
            .unwrap(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("retire failed: {}", e),
        ),
    }
}

pub async fn handle_owner_resources(path_wallet: &str, query: &str, body: Body) -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "DATABASE_URL required");
    };
    let Some(secret) = pr402::parameters::resolve_onboard_hmac_secret(pr402_db()).await else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_ONBOARD_HMAC_SECRET not set",
        );
    };

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AuthBody {
        message: String,
        signature: String,
    }

    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => String::new(),
    };

    let (message, signature) = if !body_str.is_empty() {
        let parsed: AuthBody = match serde_json::from_str(&body_str) {
            Ok(b) => b,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e));
            }
        };
        (parsed.message, parsed.signature)
    } else {
        (
            query_param(query, "message"),
            query_param(query, "signature"),
        )
    };

    if message.is_empty() || signature.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Required: message, signature (signed challenge from /resources/register/challenge)",
        );
    }

    if let Err(e) = pr402::onboard_auth::verify_onboard_submission(
        secret.as_bytes(),
        path_wallet,
        &message,
        &signature,
    ) {
        return error_response(StatusCode::UNAUTHORIZED, &e);
    }

    match db.list_owner_resources(path_wallet).await {
        Ok(entries) => json_ok_owner_list(entries),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("list failed: {}", e),
        ),
    }
}

fn json_ok_owner_list(entries: Vec<OwnerResourceEntry>) -> Response<Body> {
    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(
            serde_json::json!({ "entries": entries }).to_string(),
        ))
        .unwrap()
}

pub async fn handle_public_resources_list(query: &str) -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Public resource directory requires DATABASE_URL.",
        );
    };

    let limit: i64 = query_param(query, "limit")
        .parse::<i64>()
        .ok()
        .map(|n| n.clamp(1, 100))
        .unwrap_or(50);

    let cursor_str = query_param(query, "cursor");
    let cursor = if cursor_str.is_empty() {
        None
    } else {
        match parse_rfc3339_to_system_time(&cursor_str) {
            Ok(t) => Some(t),
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!("Invalid cursor (RFC3339): {}", e),
                );
            }
        }
    };

    let q = query_param(query, "q");
    let q_opt = if q.is_empty() { None } else { Some(q.as_str()) };
    let category = query_param(query, "category");
    let cat_opt = if category.is_empty() {
        None
    } else {
        Some(category.as_str())
    };
    let scheme = query_param(query, "scheme");
    let scheme_opt = if scheme.is_empty() {
        None
    } else {
        Some(scheme.as_str())
    };
    let tag = query_param(query, "tag");
    let tag_opt = if tag.is_empty() {
        None
    } else {
        Some(tag.as_str())
    };

    match db
        .list_public_resources(limit, cursor, q_opt, cat_opt, scheme_opt, tag_opt)
        .await
    {
        Ok(entries) => {
            let next_cursor = entries.last().map(|e| e.updated_at.clone());
            facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::Text(
                    serde_json::json!({
                        "entries": entries,
                        "nextCursor": next_cursor,
                        "notice": RESOURCE_DIRECTORY_DISCLAIMER,
                    })
                    .to_string(),
                ))
                .unwrap()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("list_public_resources failed: {}", e),
        ),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceProbeBody {
    wallet: String,
    message: String,
    signature: String,
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    resource_url: Option<String>,
}

pub async fn handle_resource_probe(body: Body) -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "DATABASE_URL required");
    };
    let Some(secret) = pr402::parameters::resolve_onboard_hmac_secret(pr402_db()).await else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_ONBOARD_HMAC_SECRET not set",
        );
    };

    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let parsed: ResourceProbeBody = match serde_json::from_str(&body_str) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };

    if let Err(e) = pr402::onboard_auth::verify_onboard_submission(
        secret.as_bytes(),
        &parsed.wallet,
        &parsed.message,
        &parsed.signature,
    ) {
        return error_response(StatusCode::UNAUTHORIZED, &e);
    }

    let probe_id = if let Some(id) = parsed.id {
        id
    } else if let Some(url) = parsed.resource_url.as_deref().filter(|s| !s.is_empty()) {
        match db.get_payable_resource_row_by_url(url).await {
            Ok(Some((id, _))) => id,
            Ok(None) => return error_response(StatusCode::NOT_FOUND, "Resource URL not found"),
            Err(e) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("lookup failed: {}", e),
                );
            }
        }
    } else {
        return error_response(StatusCode::BAD_REQUEST, "Provide id or resourceUrl");
    };

    let (owner_wallet, resource_url, http_method) =
        match db.get_payable_resource_row(probe_id).await {
            Ok(Some(row)) => row,
            Ok(None) => return error_response(StatusCode::NOT_FOUND, "Resource not found"),
            Err(e) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("lookup failed: {}", e),
                );
            }
        };

    if owner_wallet != parsed.wallet {
        return error_response(
            StatusCode::FORBIDDEN,
            "You can only probe resources registered to your wallet.",
        );
    }

    let probe = pr402::resource_probe::probe_resource_url(&http_method, &resource_url).await;
    let status_i32 = probe.http_status.map(|s| s as i32);
    let _ = db
        .record_resource_probe(
            probe_id,
            probe.ok,
            status_i32,
            probe.scheme.as_deref(),
            probe.error.as_deref(),
        )
        .await;

    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(
            serde_json::json!({
                "ok": probe.ok,
                "httpStatus": probe.http_status,
                "scheme": probe.scheme,
                "error": probe.error,
            })
            .to_string(),
        ))
        .unwrap()
}
