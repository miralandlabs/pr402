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
            "Missing wallet parameter. For DB registration use GET .../onboard/challenge then POST .../onboard with signature.",
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
/// Body JSON: `wallet`, `asset` (`SOL` | `USDC` | `WSOL` | `USDT` | base58 mint). Idempotent per `(wallet, asset)`.
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
            "Missing asset (e.g. SOL, USDC, WSOL, USDT, or a base58 mint address)",
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardSubmitBody {
    wallet: String,
    message: String,
    signature: String,
    /// Declared settlement asset for `resource_providers` (one per merchant wallet). Default **USDC**.
    #[serde(default = "default_onboard_registration_asset")]
    asset: String,
}

pub async fn handle_onboard_submit(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
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
    let submit: OnboardSubmitBody = match serde_json::from_str(&body_str) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
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
