//! Vercel serverless function entrypoint for x402 facilitator.
//!
//! Like `signer-payer-serverless-copy` bins: build `Arc` state in [`main`] before
//! [`vercel_runtime::run`]. Avoid `once_cell::sync::Lazy` for the facilitator: if init panics,
//! `Lazy` stays poisoned for the lifetime of the warm Lambda.

use pr402::{chain::ChainProvider, config::Config, db::Pr402Db, facilitator::Facilitator};
use serde::Deserialize;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use tracing::warn;
use vercel_runtime::{run, Body, Request, Response, StatusCode};

/// Shared CORS headers for `/api/v1/facilitator/*` (browser preflight + cross-origin JSON).
macro_rules! facilitator_response {
    () => {
        Response::builder()
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            .header(
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization, X-Correlation-Id, X-Correlation-ID, X-API-Version",
            )
    };
}

/// `OPTIONS` preflight for cross-origin POST/GET to the facilitator API.
fn cors_preflight_response() -> Response<Body> {
    facilitator_response!()
        .status(StatusCode::NO_CONTENT)
        .header("Access-Control-Max-Age", "86400")
        .body(Body::Empty)
        .unwrap()
}

type DynFacilitator =
    Arc<dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync>;

/// Set once from [`main`] so handlers can read optional DB the same way as before.
static PR402_DB: OnceLock<Option<Pr402Db>> = OnceLock::new();

/// Shared [`ChainProvider`] for transaction-build helpers (`build-exact-payment-tx`).
static CHAIN_PROVIDER: OnceLock<Arc<ChainProvider>> = OnceLock::new();

fn with_api_version_v1(mut res: Response<Body>) -> Response<Body> {
    res.headers_mut().insert(
        http::HeaderName::from_static("x-api-version"),
        http::HeaderValue::from_static("1"),
    );
    res
}

fn chain_provider_for_build() -> Result<&'static Arc<ChainProvider>, &'static str> {
    CHAIN_PROVIDER.get().ok_or("chain provider not initialized")
}

fn pr402_db() -> Option<&'static Pr402Db> {
    PR402_DB
        .get()
        .expect("PR402_DB: set in main before run()")
        .as_ref()
}

fn init_pr402_db_from_env() -> Option<Pr402Db> {
    match Pr402Db::from_env_var("DATABASE_URL") {
        None => None,
        Some(Ok(db)) => Some(db),
        Some(Err(e)) => {
            warn!("DATABASE_URL is set but pr402 could not connect: {}", e);
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let facilitator_ready: Result<DynFacilitator, String> = (async {
        let config = Config::from_env().map_err(|e| format!("Config error: {}", e))?;
        let chain_provider = ChainProvider::from_config(&config)
            .await
            .map_err(|e| format!("Chain provider error: {}", e))?;
        let chain_arc = Arc::new(chain_provider);
        let facilitator = pr402::facilitator::FacilitatorLocal::new((*chain_arc).clone())
            .map_err(|e| format!("Facilitator error: {}", e))?;
        if CHAIN_PROVIDER.set(chain_arc).is_err() {
            return Err("CHAIN_PROVIDER: OnceLock already initialized".into());
        }
        Ok(Arc::new(facilitator) as DynFacilitator)
    })
    .await;

    if PR402_DB.set(init_pr402_db_from_env()).is_err() {
        return Err("PR402_DB: OnceLock already initialized".into());
    }

    let handler = move |req: Request| {
        let facilitator_result = facilitator_ready.clone();

        Box::pin(async move {
            let path = req.uri().path().to_string();
            let method = req.method().clone();
            let query = req.uri().query().unwrap_or_default().to_string();
            let correlation_hdr: Option<String> = req
                .headers()
                .get("X-Correlation-Id")
                .or_else(|| req.headers().get("X-Correlation-ID"))
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let body = req.into_body();

            // Get facilitator instance
            let facilitator = match facilitator_result {
                Ok(f) => f.clone(),
                Err(e) => {
                    return Ok(with_api_version_v1(
                        facilitator_response!()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .header("Content-Type", "application/json")
                            .body(Body::Text(format!(r#"{{"error":"{}"}}"#, e)))
                            .unwrap(),
                    ));
                }
            };

            let response = match (method.as_str(), path.as_str()) {
                ("OPTIONS", p) if p.starts_with("/api/v1/facilitator") => cors_preflight_response(),
                ("POST", "/api/v1/facilitator/verify") => {
                    handle_verify(facilitator.clone(), body, correlation_hdr.as_deref()).await
                }
                ("POST", "/api/v1/facilitator/settle") => {
                    handle_settle(facilitator.clone(), body, correlation_hdr.as_deref()).await
                }
                ("GET", "/api/v1/facilitator/supported")
                | ("GET", "/api/v1/facilitator/health") => {
                    handle_supported(facilitator.clone()).await
                }
                ("GET", "/api/v1/facilitator/capabilities") => {
                    handle_capabilities(facilitator.clone()).await
                }
                ("GET", "/api/v1/facilitator/onboard/challenge") => {
                    handle_onboard_challenge(&query).await
                }
                ("POST", "/api/v1/facilitator/onboard") => {
                    handle_onboard_submit(facilitator.clone(), body).await
                }
                ("GET", "/api/v1/facilitator/onboard") => {
                    handle_onboard_preview(facilitator.clone(), &query).await
                }
                ("GET", "/api/v1/facilitator/vault-snapshot") => {
                    handle_vault_snapshot(&query).await
                }
                ("POST", "/api/v1/facilitator/build-exact-payment-tx") => {
                    handle_build_exact_payment_tx(body).await
                }
                _ => facilitator_response!()
                    .status(StatusCode::NOT_FOUND)
                    .header("Content-Type", "application/json")
                    .body(Body::Text(r#"{"error":"Not found"}"#.to_string()))
                    .unwrap(),
            };
            Ok(with_api_version_v1(response))
        })
    };

    run(handler).await
}

async fn handle_verify(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
    body: Body,
    correlation_http: Option<&str>,
) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };

    let verify_request: pr402::proto::VerifyRequest = match serde_json::from_str(&body_str) {
        Ok(req) => req,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, &format!("Invalid request: {}", e));
        }
    };

    let persist_meta = verify_request.correlation_id_for_persistence(correlation_http);
    let payee = verify_request.payee_wallet();

    match facilitator.verify(&verify_request).await {
        Ok(response) => {
            let effective_cid = persist_meta.clone().or_else(|| {
                if pr402_db().is_some() && payee.is_some() {
                    Some(pr402::payment_attempt::mint_correlation_id())
                } else {
                    None
                }
            });
            if let (Some(db), Some(cid), Some(wallet)) =
                (pr402_db(), effective_cid.as_deref(), payee.as_deref())
            {
                if let Err(e) = db.record_payment_verify(cid, wallet, true, None).await {
                    warn!(error = %e, "record_payment_verify skipped");
                }
            }
            let mut json = response.into_json();
            if let Some(ref cid) = effective_cid {
                pr402::payment_attempt::merge_correlation_into_value(&mut json, cid);
            }
            let mut res = facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json");
            if let Some(ref cid) = effective_cid {
                res = res.header("X-Correlation-Id", cid);
            }
            res.body(Body::Text(serde_json::to_string(&json).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap()
        }
        Err(e) => {
            if let (Some(db), Some(cid), Some(wallet)) =
                (pr402_db(), persist_meta.as_deref(), payee.as_deref())
            {
                let msg = format!("{}", e);
                if let Err(err) = db
                    .record_payment_verify(cid, wallet, false, Some(&msg))
                    .await
                {
                    warn!(error = %err, "record_payment_verify skipped");
                }
            }
            error_response_with_optional_correlation(
                StatusCode::BAD_REQUEST,
                &format!("Verification failed: {}", e),
                persist_meta.as_deref(),
            )
        }
    }
}

async fn handle_settle(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
    body: Body,
    correlation_http: Option<&str>,
) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };

    let settle_request: pr402::proto::SettleRequest = match serde_json::from_str(&body_str) {
        Ok(req) => req,
        Err(e) => {
            return error_response(StatusCode::BAD_REQUEST, &format!("Invalid request: {}", e));
        }
    };

    let persist_meta = settle_request.correlation_id_for_persistence(correlation_http);
    let payee = settle_request.payee_wallet();

    pr402::parameters::refresh_parameters_from_db(pr402_db()).await;

    match facilitator.settle(&settle_request).await {
        Ok(response) => {
            let mut json = response.into_json();
            if let Some(cid) = persist_meta.as_deref() {
                pr402::payment_attempt::merge_correlation_into_value(&mut json, cid);
            }
            let sig = json
                .get("transaction")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if let (Some(db), Some(cid), Some(wallet)) =
                (pr402_db(), persist_meta.as_deref(), payee.as_deref())
            {
                if let Err(e) = db
                    .record_payment_settle(cid, wallet, true, None, sig.as_deref())
                    .await
                {
                    warn!(error = %e, "record_payment_settle skipped");
                }
            }
            let mut res = facilitator_response!()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json");
            if let Some(cid) = persist_meta.as_deref() {
                res = res.header("X-Correlation-Id", cid);
            }
            res.body(Body::Text(serde_json::to_string(&json).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap()
        }
        Err(e) => {
            if let (Some(db), Some(cid), Some(wallet)) =
                (pr402_db(), persist_meta.as_deref(), payee.as_deref())
            {
                let msg = format!("{}", e);
                if let Err(err) = db
                    .record_payment_settle(cid, wallet, false, Some(&msg), None)
                    .await
                {
                    warn!(error = %err, "record_payment_settle skipped");
                }
            }
            error_response_with_optional_correlation(
                StatusCode::BAD_REQUEST,
                &format!("Settlement failed: {}", e),
                persist_meta.as_deref(),
            )
        }
    }
}

async fn handle_supported(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
) -> Response<Body> {
    match facilitator.supported().await {
        Ok(response) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(serde_json::to_string(&response).unwrap_or_else(
                |_| r#"{"error":"serialization failed"}"#.to_string(),
            )))
            .unwrap(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error: {}", e)),
    }
}

/// Stable discovery document for agents / dashboards (machine-readable complement to `/supported`).
async fn handle_capabilities(
    facilitator: Arc<
        dyn Facilitator<Error = pr402::facilitator::FacilitatorLocalError> + Send + Sync,
    >,
) -> Response<Body> {
    let supported = match facilitator.supported().await {
        Ok(s) => s,
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &format!("Error: {}", e))
        }
    };
    let supported_json = match serde_json::to_value(&supported) {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("serialization failed: {}", e),
            );
        }
    };

    let (chain_id, fee_payer, universal_settle, sla_escrow) = if let Some(cp) = CHAIN_PROVIDER.get()
    {
        (
            cp.solana.chain_id().to_string(),
            cp.solana.fee_payer().to_string(),
            cp.solana.universalsettle().is_some(),
            cp.solana.sla_escrow().is_some(),
        )
    } else {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "chain provider not initialized",
        );
    };

    let body = serde_json::json!({
        "schemaVersion": "1",
        "x402Version": 2,
        "name": "pr402 facilitator",
        "chainId": chain_id,
        "feePayer": fee_payer,
        "supported": supported_json,
        "features": {
            "universalSettleExact": universal_settle,
            "slaEscrow": sla_escrow,
            "unsignedExactPaymentTxBuild": true
        },
        "httpEndpoints": {
            "verify": { "method": "POST", "path": "/api/v1/facilitator/verify" },
            "settle": { "method": "POST", "path": "/api/v1/facilitator/settle" },
            "buildExactPaymentTx": { "method": "POST", "path": "/api/v1/facilitator/build-exact-payment-tx" },
            "supported": { "method": "GET", "path": "/api/v1/facilitator/supported" },
            "health": { "method": "GET", "path": "/api/v1/facilitator/health" },
            "capabilities": { "method": "GET", "path": "/api/v1/facilitator/capabilities" }
        },
        "specification": {
            "x402V2": "https://github.com/coinbase/x402/blob/main/specs/x402-specification-v2.md"
        }
    });

    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(body.to_string()))
        .unwrap()
}

/// Public PDA preview only (no DB). Use challenge + POST `/onboard` to persist with proof-of-control.
async fn handle_onboard_preview(
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

async fn handle_onboard_challenge(query: &str) -> Response<Body> {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OnboardSubmitBody {
    wallet: String,
    message: String,
    signature: String,
}

async fn handle_onboard_submit(
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

    match facilitator.onboard(&submit.wallet).await {
        Ok(response) => {
            if let Some(db) = pr402_db() {
                if let Some(info) = response.schemes.get("exact") {
                    if let Err(e) = db
                        .upsert_resource_provider_vaults_verified(
                            &submit.wallet,
                            "native_sol",
                            None,
                            &info.vault_pda,
                            &info.sol_storage_pda,
                        )
                        .await
                    {
                        warn!(error = %e, "persist verified onboard vaults skipped");
                    }
                }
            } else {
                warn!("DATABASE_URL unset; onboard signature accepted but resource_providers not persisted");
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

/// RPC-only UniversalSettle vault snapshot (no tx fees). Query: `wallet=` & optional `spl_mint=`,
/// optional `spl_token_program=` (Token-2022 mints; default legacy Token),
/// optional `spl_balance_scope=vault_ata` (default) or `owner_wallet` (`getTokenAccountsByOwner`, cf. spl-token-balance-serverless).
async fn handle_vault_snapshot(query: &str) -> Response<Body> {
    let wallet = query_param(query, "wallet");
    let spl_mint = query_param(query, "spl_mint");
    let spl_token_program_raw = query_param(query, "spl_token_program");
    let spl_scope = query_param(query, "spl_balance_scope");
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
    let Some(us) = cfg.universalsettle.as_ref() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "UNIVERSALSETTLE_PROGRAM_ID not configured",
        );
    };
    let seller = match pr402::vault_balance::parse_seller(&wallet) {
        Ok(p) => p,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
    };
    let mint_opt = if spl_mint.is_empty() {
        None
    } else {
        match solana_pubkey::Pubkey::from_str(&spl_mint) {
            Ok(m) => Some(m),
            Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid spl_mint"),
        }
    };
    let token_prog_opt = if spl_token_program_raw.is_empty() {
        None
    } else {
        match solana_pubkey::Pubkey::from_str(&spl_token_program_raw) {
            Ok(p) => Some(p),
            Err(_) => {
                return error_response(StatusCode::BAD_REQUEST, "Invalid spl_token_program");
            }
        }
    };
    let rpc =
        solana_client::nonblocking::rpc_client::RpcClient::new(cfg.solana_rpc_url.to_string());
    let mut snap = match pr402::vault_balance::fetch_universalsettle_vault_snapshot(
        &rpc,
        us.program_id,
        seller,
        mint_opt,
        token_prog_opt,
    )
    .await
    {
        Ok(s) => s,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
    };

    let owner_wallet_scope = spl_scope == "owner_wallet"
        || spl_scope == "owner_mint"
        || spl_scope == "get_token_accounts_by_owner";

    if owner_wallet_scope {
        if let Some(mint) = mint_opt {
            match pr402::vault_balance::fetch_spl_raw_balance_by_owner_and_mint(
                &rpc, &seller, &mint,
            )
            .await
            {
                Ok((raw, dec)) => {
                    snap.spl_amount_raw = raw;
                    snap.spl_decimals = dec.or(snap.spl_decimals);
                }
                Err(e) => return error_response(StatusCode::BAD_REQUEST, &e),
            }
        }
    }

    let spl_scope_out = if owner_wallet_scope && mint_opt.is_some() {
        "owner_wallet"
    } else {
        "vault_ata"
    };

    let body = serde_json::json!({
        "seller": snap.seller.to_string(),
        "programId": snap.program_id.to_string(),
        "splitVaultPda": snap.split_vault_pda.to_string(),
        "vaultSolStoragePda": snap.vault_sol_storage_pda.to_string(),
        "spendableLamports": snap.spendable_lamports,
        "vaultSplAta": snap.vault_spl_ata.map(|a| a.to_string()),
        "splAmountRaw": snap.spl_amount_raw,
        "splDecimals": snap.spl_decimals,
        "splBalanceScope": spl_scope_out,
    });
    facilitator_response!()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(body.to_string()))
        .unwrap()
}

/// Build an unsigned `v2:solana:exact` SPL `TransferChecked` transaction (+ compute budget + optional
/// merchant ATA create) for wallet signing. See [`pr402::exact_payment_build`].
async fn handle_build_exact_payment_tx(body: Body) -> Response<Body> {
    let cp = match chain_provider_for_build() {
        Ok(c) => c,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => return error_response(StatusCode::BAD_REQUEST, "Missing request body"),
    };
    let req: pr402::exact_payment_build::BuildExactPaymentTxRequest =
        match serde_json::from_str(&body_str) {
            Ok(r) => r,
            Err(e) => {
                return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e))
            }
        };
    match pr402::exact_payment_build::build_exact_spl_payment_tx(&cp.solana, req).await {
        Ok(out) => facilitator_response!()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::Text(
                serde_json::to_string(&out)
                    .unwrap_or_else(|_| r#"{"error":"serialize failed"}"#.into()),
            ))
            .unwrap(),
        Err(e) => {
            let status = match e {
                pr402::exact_payment_build::ExactPaymentBuildError::NetworkMismatch { .. }
                | pr402::exact_payment_build::ExactPaymentBuildError::InvalidRequest(_) => {
                    StatusCode::BAD_REQUEST
                }
                pr402::exact_payment_build::ExactPaymentBuildError::Unsupported(_) => {
                    StatusCode::NOT_IMPLEMENTED
                }
                pr402::exact_payment_build::ExactPaymentBuildError::Rpc(_) => {
                    StatusCode::BAD_GATEWAY
                }
            };
            error_response(status, &e.to_string())
        }
    }
}

fn query_param(query: &str, key: &str) -> String {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    error_response_with_optional_correlation(status, message, None)
}

fn error_response_with_optional_correlation(
    status: StatusCode,
    message: &str,
    correlation_id: Option<&str>,
) -> Response<Body> {
    let mut body = serde_json::Map::new();
    body.insert(
        "error".to_string(),
        serde_json::Value::String(message.to_string()),
    );
    if let Some(cid) = correlation_id {
        body.insert(
            "correlationId".to_string(),
            serde_json::Value::String(cid.to_string()),
        );
    }
    let json = serde_json::Value::Object(body);
    let mut res = facilitator_response!()
        .status(status)
        .header("Content-Type", "application/json");
    if let Some(cid) = correlation_id {
        res = res.header("X-Correlation-Id", cid);
    }
    res.body(Body::Text(json.to_string())).unwrap()
}
