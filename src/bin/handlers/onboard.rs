use super::*;

use pr402::db::DbError;
use pr402::facilitator::FacilitatorLocalError;
use pr402::scheme::X402SchemeFacilitatorError;

pub async fn handle_onboard_preview(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
    query: &str,
) -> Response<Body> {
    let wallet = query_param(query, "wallet");
    if wallet.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Missing wallet parameter. For DB registration use GET /sellers/{wallet}/challenge then POST /sellers/{wallet}/register with signature.",
        );
    }

    match facilitator.onboard(&wallet).await {
        Ok(response) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(serde_json::to_string(&response).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Onboarding failed: {}", e),
        ),
    }
}

/// Agent-native seller provisioning: unsigned tx for UniversalSettle SplitVault + asset surface (SOL or SPL vault ATA).
/// Body JSON: `wallet`, `asset` (`SOL` | `USDC` | `USDT` | base58 mint). Idempotent per `(wallet, asset)`.
pub async fn handle_onboard_provision(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
    body: Body,
) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OnboardProvisionBody {
        wallet: String,
        asset: String,
    }

    let submit: OnboardProvisionBody = match serde_json::from_str(&body_str) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };
    let wallet = submit.wallet.trim();
    let asset = submit.asset.trim();
    if wallet.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Missing wallet");
    }
    if asset.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Missing asset (e.g. SOL, USDC, USDT, or a base58 mint address)",
        );
    }

    match facilitator.build_onboard_provision_tx(wallet, asset).await {
        Ok(response) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(serde_json::to_string(&response).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap(),
        Err(FacilitatorLocalError::Onboard(X402SchemeFacilitatorError::InvalidPayload(m))) => {
            error_response(StatusCode::BAD_REQUEST, &m)
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Provision build failed: {}", e),
        ),
    }
}

pub async fn handle_onboard_challenge(query: &str) -> Response<Body> {
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
            "PR402_ONBOARD_HMAC_SECRET not set (env or parameters table); see migrations/init.sql",
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

fn default_onboard_registration_asset() -> String {
    "USDC".to_string()
}

/// Optional seller-declared discovery payload, persisted into `resource_providers` when the
/// signed onboard submit succeeds. All fields are optional; callers can submit any subset
/// to update only those columns.
///
/// Size / pattern limits (enforced below; keep in sync with `docs/DISCOVERY.md`):
///   - service_url    : must start with https:// or http:// , ≤ 2048 chars
///   - display_name   : ≤ 64 chars
///   - description    : ≤ 280 chars (tweet-sized, intentionally)
///   - tags           : ≤ 5 entries; each ≤ 32 chars; lowercase [a-z0-9-]
///   - service_metadata: JSON object, serialized ≤ 4096 bytes
///   - listing_opt_in : boolean toggle; false hides the row from GET /providers
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardDiscoveryPayload {
    #[serde(default)]
    service_url: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    service_metadata: Option<serde_json::Value>,
    #[serde(default)]
    listing_opt_in: Option<bool>,
}

fn validate_discovery(d: &OnboardDiscoveryPayload) -> Result<(), String> {
    if let Some(u) = d.service_url.as_deref() {
        if u.len() > 2048 {
            return Err("discovery.serviceUrl exceeds 2048 chars".into());
        }
        if !(u.starts_with("https://") || u.starts_with("http://")) {
            return Err("discovery.serviceUrl must start with https:// or http://".into());
        }
    }
    if let Some(n) = d.display_name.as_deref() {
        if n.chars().count() > 64 {
            return Err("discovery.displayName exceeds 64 characters".into());
        }
    }
    if let Some(desc) = d.description.as_deref() {
        if desc.chars().count() > 280 {
            return Err("discovery.description exceeds 280 characters".into());
        }
    }
    if let Some(tags) = d.tags.as_deref() {
        if tags.len() > 5 {
            return Err("discovery.tags accepts at most 5 entries".into());
        }
        for t in tags {
            if t.is_empty() || t.len() > 32 {
                return Err("discovery.tags: each tag must be 1..=32 characters".into());
            }
            if !t
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                return Err("discovery.tags: lowercase ASCII letters, digits, and '-' only".into());
            }
        }
    }
    if let Some(meta) = d.service_metadata.as_ref() {
        if !meta.is_object() {
            return Err("discovery.serviceMetadata must be a JSON object".into());
        }
        let serialized = serde_json::to_vec(meta).unwrap_or_default();
        if serialized.len() > 4096 {
            return Err("discovery.serviceMetadata exceeds 4096 bytes serialized".into());
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardSubmitBody {
    /// Optional — if omitted, the path segment `/sellers/{wallet}/register` is canonical.
    /// When present, must equal the path wallet.
    #[serde(default)]
    wallet: Option<String>,
    message: String,
    signature: String,
    /// Declared settlement asset for `resource_providers` (one per merchant wallet). Default **USDC**.
    #[serde(default = "default_onboard_registration_asset")]
    asset: String,
    /// Optional seller-declared discovery payload — only applied after the signature
    /// verifies AND the vault is on-chain. Absent = no change to existing columns.
    #[serde(default)]
    discovery: Option<OnboardDiscoveryPayload>,
}

/// Submit body after merging the path wallet with the (optional) body wallet.
struct SubmitFields {
    wallet: String,
    message: String,
    signature: String,
    asset: String,
    discovery: Option<OnboardDiscoveryPayload>,
}

pub async fn handle_onboard_submit(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
    path_wallet: &str,
    body: Body,
) -> Response<Body> {
    let Some(secret) = pr402::parameters::resolve_onboard_hmac_secret(pr402_db()).await else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_ONBOARD_HMAC_SECRET not set (env or parameters table); see migrations/init.sql",
        );
    };

    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let parsed: OnboardSubmitBody = match serde_json::from_str(&body_str) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };
    let wallet = match parsed.wallet.as_deref().map(str::trim) {
        Some(w) if !w.is_empty() => {
            if w != path_wallet {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "Body `wallet` does not match the path segment; omit it or set it equal to the URL wallet.",
                );
            }
            w.to_string()
        }
        _ => path_wallet.to_string(),
    };
    let submit = SubmitFields {
        wallet,
        message: parsed.message,
        signature: parsed.signature,
        asset: parsed.asset,
        discovery: parsed.discovery,
    };
    if let Err(e) = pr402::onboard_auth::verify_onboard_submission(
        secret.as_bytes(),
        &submit.wallet,
        &submit.message,
        &submit.signature,
    ) {
        return error_response(StatusCode::UNAUTHORIZED, &e);
    }

    let devnet = CHAIN_PROVIDER
        .get()
        .map(|cp| pr402::seller_provision::cluster_is_devnet(cp.solana.as_ref()))
        .unwrap_or(false);
    let resolved = match pr402::seller_provision::resolve_seller_asset(submit.asset.trim(), devnet)
    {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let (reg_mode, reg_mint_owned) =
        pr402::seller_provision::resolved_seller_asset_to_settlement_rail(&resolved);
    let mint_for_allowlist = match &resolved {
        pr402::seller_provision::ResolvedSellerAsset::NativeSol => {
            pr402::seller_provision::NATIVE_SOL_ASSET_MINT
        }
        pr402::seller_provision::ResolvedSellerAsset::Spl { mint, .. } => *mint,
    };
    if let Some(db) = pr402_db() {
        if let Err(msg) =
            pr402::parameters::ensure_allowed_payment_mint(Some(db), &mint_for_allowlist).await
        {
            return error_response(StatusCode::BAD_REQUEST, &msg);
        }
    }

    match facilitator.onboard(&submit.wallet).await {
        Ok(response) => {
            if let Some(db) = pr402_db() {
                if let Some(info) = response.schemes.get("exact") {
                    // Gate: refuse to persist a `resource_providers` row that points at a vault
                    // PDA which does not yet exist on-chain. Without this check the registry can
                    // acquire dormant rows whose `split_vault_pda` is only a mathematical address
                    // — buyers would then try to pay a non-existent account.
                    //
                    // This is belt-and-suspenders; the well-lit path for sellers is:
                    //   1. POST /sellers/provision-tx                  → sign → broadcast  (the "activate" step)
                    //   2. GET  /sellers/{wallet}/challenge            (the "verify identity" step)
                    //   3. POST /sellers/{wallet}/register (this handler)
                    if let Some(cp) = CHAIN_PROVIDER.get() {
                        let vault_pk = match solana_pubkey::Pubkey::from_str(&info.vault_pda) {
                            Ok(pk) => pk,
                            Err(e) => {
                                return error_response(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    &format!("internal: invalid vault pda: {}", e),
                                );
                            }
                        };
                        match cp.solana.account_exists(&vault_pk).await {
                            Ok(true) => {}
                            Ok(false) => {
                                return error_response(
                                    StatusCode::CONFLICT,
                                    "Vault is not yet on-chain for this wallet. Activate first: \
                                     POST /api/v1/facilitator/sellers/provision-tx (sign and broadcast), \
                                     then retry registry submit.",
                                );
                            }
                            Err(e) => {
                                warn!(
                                    target: LOG_SERVER_LOG,
                                    error = %e,
                                    "onboard submit: vault existence probe failed; refusing to persist"
                                );
                                return error_response(
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    "Could not verify vault on-chain right now; try again shortly.",
                                );
                            }
                        }
                    } else {
                        warn!(
                            target: LOG_SERVER_LOG,
                            "onboard submit: CHAIN_PROVIDER not initialized; cannot verify vault existence"
                        );
                        return error_response(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "Chain provider not initialized; registry submit unavailable.",
                        );
                    }

                    match db
                        .upsert_resource_provider_vaults_verified(
                            &submit.wallet,
                            reg_mode,
                            reg_mint_owned.as_deref(),
                            &info.vault_pda,
                            &info.sol_storage_pda,
                        )
                        .await
                    {
                        Ok(_) => {}
                        Err(DbError::FacilitatorPolicy(msg)) => {
                            return error_response(StatusCode::BAD_REQUEST, &msg);
                        }
                        Err(e) => {
                            warn!(target: LOG_SERVER_LOG, error = %e, "persist verified onboard vaults skipped");
                        }
                    }

                    // Optional discovery payload: validated first so we surface shape
                    // errors as 400s before touching the DB. Writing happens in its own
                    // UPDATE against the same wallet so the verified row(s) and their
                    // public metadata stay in lockstep.
                    if let Some(discovery) = &submit.discovery {
                        if let Err(msg) = validate_discovery(discovery) {
                            return error_response(StatusCode::BAD_REQUEST, &msg);
                        }
                        match db
                            .apply_seller_discovery(
                                &submit.wallet,
                                discovery.service_url.as_deref(),
                                discovery.display_name.as_deref(),
                                discovery.description.as_deref(),
                                discovery.tags.as_deref(),
                                discovery.service_metadata.as_ref(),
                                discovery.listing_opt_in,
                            )
                            .await
                        {
                            Ok(0) => {
                                // UPDATE matched zero rows despite a payload being submitted.
                                // This almost always means the row is still retired (stale binary
                                // without the retirement-clearing upsert fix), or the row was
                                // deleted between upsert and this update. Loud warning so the
                                // silent-success trap stops biting operators.
                                warn!(
                                    target: LOG_SERVER_LOG,
                                    wallet = %submit.wallet,
                                    "apply_seller_discovery: 0 rows updated (discovery payload ignored). \
                                     Likely cause: row is still retired or missing. \
                                     Confirm upsert cleared retired_at/inactive."
                                );
                            }
                            Ok(n) => {
                                info!(
                                    target: LOG_SERVER_LOG,
                                    wallet = %submit.wallet,
                                    rows = n,
                                    "apply_seller_discovery: discovery payload applied"
                                );
                            }
                            Err(e) => {
                                warn!(
                                    target: LOG_SERVER_LOG,
                                    error = %e,
                                    "apply_seller_discovery failed; onboard write succeeded but discovery fields not persisted"
                                );
                            }
                        }
                    }
                }
            } else {
                warn!(
                    target: LOG_SERVER_LOG,
                    "DATABASE_URL unset; onboard signature accepted but resource_providers not persisted"
                );
            }
            facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::Text(serde_json::to_string(&response).unwrap_or_else(
                    |_| r#"{"error":"serialization failed"}"#.to_string(),
                )))
                .unwrap()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Onboarding failed: {}", e),
        ),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardRetireBody {
    /// Optional — if omitted, the path segment `/sellers/{wallet}/retire` is canonical.
    /// When present, must equal the path wallet.
    #[serde(default)]
    wallet: Option<String>,
    message: String,
    signature: String,
}

/// Opt-out endpoint: retire all `resource_providers` rows for a wallet.
///
/// Uses the same HMAC challenge + wallet signature flow as `POST /sellers/{wallet}/register`,
/// but flips `retired_at` / `inactive` / `listing_opt_in` so the rows stop appearing in
/// discovery and future settle attempts warn. The signed `message` must have been obtained
/// from `GET /sellers/{wallet}/challenge`; the server-side HMAC binds the message to the
/// deployment and the wallet proves key control. No on-chain write is issued — this is
/// an off-chain registry retirement only.
pub async fn handle_onboard_retire(path_wallet: &str, body: Body) -> Response<Body> {
    let Some(secret) = pr402::parameters::resolve_onboard_hmac_secret(pr402_db()).await else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_ONBOARD_HMAC_SECRET not set (env or parameters table); see migrations/init.sql",
        );
    };

    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let parsed: OnboardRetireBody = match serde_json::from_str(&body_str) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };
    let wallet = match parsed.wallet.as_deref().map(str::trim) {
        Some(w) if !w.is_empty() => {
            if w != path_wallet {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "Body `wallet` does not match the path segment; omit it or set it equal to the URL wallet.",
                );
            }
            w.to_string()
        }
        _ => path_wallet.to_string(),
    };
    if let Err(e) = pr402::onboard_auth::verify_onboard_submission(
        secret.as_bytes(),
        &wallet,
        &parsed.message,
        &parsed.signature,
    ) {
        return error_response(StatusCode::UNAUTHORIZED, &e);
    }

    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Facilitator registry is not enabled (DATABASE_URL unset); retirement is a no-op here.",
        );
    };

    match db.retire_resource_provider(&wallet).await {
        Ok(updated) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(
                serde_json::json!({
                    "wallet": wallet,
                    "retiredRowCount": updated,
                    "note": if updated == 0 {
                        "No active rows found for this wallet; nothing to retire."
                    } else {
                        "Wallet retired from the off-chain registry. On-chain vault state is unchanged; existing payments are still settleable."
                    }
                })
                .to_string(),
            ))
            .unwrap(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("Retirement failed: {}", e),
        ),
    }
}

/// Public seller directory: list pages of verified, opted-in, unretired sellers. Honors
/// two query params — `limit` (clamped 1..=100) and `cursor` (RFC3339 timestamp; rows are
/// returned with `updated_at < cursor` so clients page backwards by passing the previous
/// page's last `updatedAt`).
///
/// Deployments without a database respond with 503.
pub async fn handle_public_providers_list(query: &str) -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Public directory requires DATABASE_URL to be configured.",
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
                    &format!("Invalid `cursor` (expected RFC3339): {}", e),
                );
            }
        }
    };

    match db.list_public_providers(limit, cursor).await {
        Ok(entries) => {
            let next_cursor = entries.last().map(|e| e.updated_at.clone());
            let body = serde_json::json!({
                "entries": entries,
                "nextCursor": next_cursor,
                "notice": DIRECTORY_DISCLAIMER,
            });
            facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::Text(body.to_string()))
                .unwrap()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("list_public_providers failed: {}", e),
        ),
    }
}

/// Public seller directory: lookup one wallet. 404 when the row doesn't exist or isn't
/// opted into the public listing.
pub async fn handle_public_provider_single(wallet: &str) -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Public directory requires DATABASE_URL to be configured.",
        );
    };
    match db.get_public_provider(wallet).await {
        Ok(Some(entry)) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(
                serde_json::json!({
                    "entry": entry,
                    "notice": DIRECTORY_DISCLAIMER,
                })
                .to_string(),
            ))
            .unwrap(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "No public listing for this wallet."),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("get_public_provider failed: {}", e),
        ),
    }
}

/// Seller payment history: returns the most recent `payment_attempts` rows for a wallet,
/// joined through the resource-provider row. Intended for seller dashboards. Requires the
/// same HMAC challenge + wallet-signed message as `POST /sellers/{wallet}/register` so only
/// the wallet owner can read their own history. Query params: `message`, `signature`
/// (wallet-signed challenge), `limit` (1..=100), `cursor` (RFC3339 createdAt).
pub async fn handle_seller_payments_list(query: &str, body: Body) -> Response<Body> {
    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Payment history requires DATABASE_URL to be configured.",
        );
    };
    let Some(secret) = pr402::parameters::resolve_onboard_hmac_secret(pr402_db()).await else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_ONBOARD_HMAC_SECRET not set (env or parameters table); payment history not available.",
        );
    };

    // Accept the challenge + signature either in the POST body (preferred, keeps long
    // signatures out of URL query strings) or as query params for quick browser testing.
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct BodyForm {
        wallet: String,
        message: String,
        signature: String,
        #[serde(default)]
        limit: Option<i64>,
        #[serde(default)]
        cursor: Option<String>,
    }

    let (wallet, message, signature, body_limit, body_cursor) = match body {
        Body::Text(ref s) if !s.is_empty() => {
            let parsed: BodyForm = match serde_json::from_str(s) {
                Ok(b) => b,
                Err(e) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        &format!("Invalid JSON: {}", e),
                    );
                }
            };
            (
                parsed.wallet,
                parsed.message,
                parsed.signature,
                parsed.limit,
                parsed.cursor,
            )
        }
        _ => (
            query_param(query, "wallet"),
            query_param(query, "message"),
            query_param(query, "signature"),
            None,
            None,
        ),
    };
    if wallet.is_empty() || message.is_empty() || signature.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Required: wallet, message, signature (POST JSON body preferred).",
        );
    }

    if let Err(e) = pr402::onboard_auth::verify_onboard_submission(
        secret.as_bytes(),
        &wallet,
        &message,
        &signature,
    ) {
        return error_response(StatusCode::UNAUTHORIZED, &e);
    }

    let limit = body_limit
        .or_else(|| query_param(query, "limit").parse::<i64>().ok())
        .unwrap_or(50)
        .clamp(1, 100);

    let cursor_str = body_cursor.unwrap_or_else(|| query_param(query, "cursor"));
    let cursor = if cursor_str.is_empty() {
        None
    } else {
        match parse_rfc3339_to_system_time(&cursor_str) {
            Ok(t) => Some(t),
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!("Invalid `cursor` (expected RFC3339): {}", e),
                );
            }
        }
    };

    match db.list_seller_payments(&wallet, limit, cursor).await {
        Ok(entries) => {
            let next_cursor = entries.last().map(|e| e.created_at.clone());
            facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::Text(
                    serde_json::json!({
                        "entries": entries,
                        "nextCursor": next_cursor,
                    })
                    .to_string(),
                ))
                .unwrap()
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("list_seller_payments failed: {}", e),
        ),
    }
}

/// Minimal RFC3339 parser backed by `SystemTime`. Accepts the subset we actually emit
/// (`YYYY-MM-DDTHH:MM:SS[.ffffff]Z`). No dep on `chrono` / `time`.
fn parse_rfc3339_to_system_time(s: &str) -> Result<std::time::SystemTime, String> {
    // Expected: 1970-01-01T00:00:00.000000Z or without fractional seconds.
    let s = s.trim();
    if !s.ends_with('Z') {
        return Err("must end with 'Z'".into());
    }
    let body = &s[..s.len() - 1];
    let (date, time_part) = body
        .split_once('T')
        .ok_or_else(|| "missing 'T' separator".to_string())?;
    let (y, m, d) = {
        let mut it = date.split('-');
        let y: i64 = it
            .next()
            .ok_or("date")?
            .parse()
            .map_err(|_| "year".to_string())?;
        let m: u32 = it
            .next()
            .ok_or("date")?
            .parse()
            .map_err(|_| "month".to_string())?;
        let d: u32 = it
            .next()
            .ok_or("date")?
            .parse()
            .map_err(|_| "day".to_string())?;
        (y, m, d)
    };
    let (hhmmss, frac) = match time_part.split_once('.') {
        Some((a, b)) => (a, b),
        None => (time_part, "0"),
    };
    let mut hms = hhmmss.split(':');
    let hh: u32 = hms
        .next()
        .ok_or("time")?
        .parse()
        .map_err(|_| "hour".to_string())?;
    let mm: u32 = hms
        .next()
        .ok_or("time")?
        .parse()
        .map_err(|_| "minute".to_string())?;
    let ss: u32 = hms
        .next()
        .ok_or("time")?
        .parse()
        .map_err(|_| "second".to_string())?;
    // Normalize fractional to nanoseconds.
    let frac_trim: String = frac.chars().take(9).collect();
    let frac_padded = format!("{:0<9}", frac_trim);
    let nanos: u32 = frac_padded
        .parse::<u32>()
        .map_err(|_| "fractional seconds".to_string())?;

    // days_from_civil: Howard Hinnant (public domain / CC0).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64; // [0, 399]
    let doy = {
        let m = m as i64;
        let d = d as i64;
        let shifted_m = if m > 2 { m - 3 } else { m + 9 };
        ((153 * shifted_m + 2) / 5 + d - 1) as u64
    };
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    let total_secs = days * 86_400 + hh as i64 * 3600 + mm as i64 * 60 + ss as i64;
    if total_secs < 0 {
        return Err("timestamp before 1970 not supported".into());
    }
    Ok(std::time::UNIX_EPOCH + std::time::Duration::new(total_secs as u64, nanos))
}

const DIRECTORY_DISCLAIMER: &str = "Facilitator verifies wallet control only. Listing does not imply endorsement, audit, or vetting of the advertised service.";

#[cfg(test)]
mod tests {
    use super::{parse_rfc3339_to_system_time, validate_discovery, OnboardDiscoveryPayload};

    fn sample() -> OnboardDiscoveryPayload {
        OnboardDiscoveryPayload {
            service_url: Some("https://api.example.com/x402".into()),
            display_name: Some("Example Oracle".into()),
            description: Some("Weather API gated on x402".into()),
            tags: Some(vec!["weather".into(), "api".into()]),
            service_metadata: Some(serde_json::json!({ "pricing": "per-call" })),
            listing_opt_in: Some(true),
        }
    }

    #[test]
    fn validate_discovery_accepts_reasonable_payload() {
        validate_discovery(&sample()).unwrap();
    }

    #[test]
    fn validate_discovery_rejects_oversized_description() {
        let mut d = sample();
        d.description = Some("x".repeat(281));
        assert!(validate_discovery(&d).is_err());
    }

    #[test]
    fn validate_discovery_rejects_http_without_scheme() {
        let mut d = sample();
        d.service_url = Some("api.example.com".into());
        assert!(validate_discovery(&d).is_err());
    }

    #[test]
    fn validate_discovery_rejects_bad_tags() {
        let mut d = sample();
        d.tags = Some(vec!["Bad Tag!".into()]);
        assert!(validate_discovery(&d).is_err());

        let mut d2 = sample();
        d2.tags = Some(vec!["a".into(); 6]);
        assert!(validate_discovery(&d2).is_err());
    }

    #[test]
    fn validate_discovery_rejects_non_object_metadata() {
        let mut d = sample();
        d.service_metadata = Some(serde_json::json!([1, 2, 3]));
        assert!(validate_discovery(&d).is_err());
    }

    #[test]
    fn parse_rfc3339_roundtrips_epoch() {
        let t0 = parse_rfc3339_to_system_time("1970-01-01T00:00:00.000000Z").unwrap();
        assert_eq!(t0, std::time::UNIX_EPOCH);
    }

    #[test]
    fn parse_rfc3339_accepts_without_fractional() {
        let t = parse_rfc3339_to_system_time("2026-05-10T12:34:56Z").unwrap();
        let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap();
        // 2026-05-10 12:34:56 UTC is 1778416496 seconds after epoch.
        assert_eq!(dur.as_secs(), 1778416496);
    }

    #[test]
    fn parse_rfc3339_rejects_missing_z() {
        assert!(parse_rfc3339_to_system_time("2026-05-10T00:00:00").is_err());
    }
}
