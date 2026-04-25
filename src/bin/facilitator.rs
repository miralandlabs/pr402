//! Vercel serverless function entrypoint for x402 facilitator.
//!
//! Like `signer-payer-serverless-copy` bins: build state in [`main`] before [`vercel_runtime::run`].
//! DB: `signer-payer/src/route_handler.rs` calls `DATABASE.get_or_init(init_database)` before `run`;
//! pr402 sets `PR402_DB` the same way — pool from `DATABASE_URL`, mirroring `database.rs` `Database::new`.

use pr402::{
    chain::ChainProvider,
    config::Config,
    db::{
        PaymentAuditMetadata, PaymentOutcome, Pr402Db, ResourceProviderInfo, ResourceProviderRail,
    },
    facilitator::Facilitator,
    scheme::v2_solana_escrow::types::SLAEscrowScheme,
};
use serde::Deserialize;
use std::io::LineWriter;
use std::mem::size_of;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use subtle::ConstantTimeEq;
use tracing::{error, info, warn};
use tracing_log::LogTracer;
use tracing_subscriber::{fmt::format::FmtSpan, prelude::*};
use vercel_runtime::{run, Body, Request, Response, StatusCode};

/// Shared CORS headers for `/api/v1/facilitator/*` (browser preflight + cross-origin JSON).
macro_rules! facilitator_response {
    () => {
        Response::builder()
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            .header(
                "Access-Control-Allow-Headers",
                "Content-Type, Authorization, X-Correlation-ID, X-API-Version, PAYMENT-SIGNATURE, PAYMENT-RESPONSE",
            )
            .header(
                "Access-Control-Expose-Headers",
                "X-Correlation-ID, X-API-Version, PAYMENT-RESPONSE",
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

/// Shared [`ChainProvider`] for transaction-build helpers (`build-exact-payment-tx`,
/// `build-sla-escrow-payment-tx`).
static CHAIN_PROVIDER: OnceLock<Arc<ChainProvider>> = OnceLock::new();

/// Use an explicit target so `RUST_LOG=pr402=info` shows **`facilitator` bin** lines (default target is
/// `facilitator`, which does not match the `pr402` filter and hides verify/settle logs).
/// Institutional audit log category (mirrors signer-payer baseline).
const LOG_SERVER_LOG: &str = "server_log";
const SCHEMA_VERSION: &str = "1.0.0";

// ── INFRA-4: Lightweight in-process rate limiter for build endpoints ─────────
use std::sync::Mutex;
static BUILD_RATE_LIMITER: OnceLock<Mutex<BuildRateState>> = OnceLock::new();

struct BuildRateState {
    /// Start of the current sliding window (UNIX seconds).
    window_start: u64,
    /// Request count in the current window.
    count: u64,
    /// Max requests per window (loaded from env once).
    limit: u64,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SweepRequest {
    limit: Option<u64>,
    cooldown_seconds: Option<u64>,
    require_recent_settle_within_seconds: Option<u64>,
    dry_run: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SweepItemResult {
    wallet: String,
    settlement_mode: String,
    spl_mint: Option<String>,
    available_raw: u64,
    threshold_raw: u64,
    status: String,
    action: String,
    signature: Option<String>,
    error: Option<String>,
}

impl BuildRateState {
    fn new() -> Self {
        let limit = std::env::var("PR402_BUILD_RATE_LIMIT_PER_MIN")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60u64);
        Self {
            window_start: 0,
            count: 0,
            limit,
        }
    }
}

/// Returns `Some(Response 429)` if the build rate limit is exceeded.
fn check_build_rate_limit() -> Option<Response<Body>> {
    let state = BUILD_RATE_LIMITER.get_or_init(|| Mutex::new(BuildRateState::new()));
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Reset window every 60 seconds
    if now.saturating_sub(guard.window_start) >= 60 {
        guard.window_start = now;
        guard.count = 0;
    }
    guard.count += 1;
    if guard.count > guard.limit {
        warn!(
            target: LOG_SERVER_LOG,
            count = guard.count,
            limit = guard.limit,
            "build endpoint rate limit exceeded"
        );
        Some(
            facilitator_response!()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header("Content-Type", "application/json")
                .header("Retry-After", "60")
                .body(Body::Text(
                    r#"{"error":"Rate limit exceeded for build endpoints. Retry after 60s."}"#
                        .to_string(),
                ))
                .unwrap(),
        )
    } else {
        None
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    schema_version: &'static str,
    database: &'static str,
    solana_rpc: &'static str,
    solana_slot: Option<u64>,
    environment: String,
    solana_network: String,
}

/// Typed capabilities discovery response — stabilizes the contract for agents.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesResponse {
    schema_version: &'static str,
    x402_version: u8,
    name: &'static str,
    chain_id: String,
    fee_payer: String,
    supported: serde_json::Value,
    features: CapabilitiesFeatures,
    http_endpoints: CapabilitiesHttpEndpoints,
    agent_manifest: AgentManifest,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesFeatures {
    universal_settle_exact: bool,
    sla_escrow: bool,
    unsigned_exact_payment_tx_build: bool,
    unsigned_sla_escrow_payment_tx_build: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HttpEndpointInfo {
    method: &'static str,
    path: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth: Option<&'static str>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesHttpEndpoints {
    verify: HttpEndpointInfo,
    settle: HttpEndpointInfo,
    build_exact_payment_tx: HttpEndpointInfo,
    build_sla_escrow_payment_tx: HttpEndpointInfo,
    sweep: HttpEndpointInfo,
    sweep_cron: HttpEndpointInfo,
    onboard: HttpEndpointInfo,
    build_onboard_tx: HttpEndpointInfo,
    supported: HttpEndpointInfo,
    health: HttpEndpointInfo,
    capabilities: HttpEndpointInfo,
    discovery: HttpEndpointInfo,
    upgrade: HttpEndpointInfo,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentManifest {
    open_api: &'static str,
    pay_to_semantics: &'static str,
    integration_guide: &'static str,
    seller_quick_start: &'static str,
    seller_onboarding_guide: &'static str,
    buyer_quick_start: &'static str,
    x402_spec: &'static str,
}

fn with_api_version_v1(mut res: Response<Body>, correlation_id: Option<&str>) -> Response<Body> {
    res.headers_mut().insert(
        http::HeaderName::from_static("x-api-version"),
        http::HeaderValue::from_static("1"),
    );
    res.headers_mut().insert(
        http::HeaderName::from_static("x-schema-version"),
        http::HeaderValue::from_static(SCHEMA_VERSION),
    );
    if let Some(cid) = correlation_id {
        if let Ok(hv) = http::HeaderValue::from_str(cid) {
            res.headers_mut()
                .insert(http::HeaderName::from_static("X-Correlation-ID"), hv);
        }
    }
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

fn payment_scheme_is_sla_escrow(scheme: Option<&str>) -> bool {
    scheme == Some(SLAEscrowScheme.as_ref())
}

/// Runs after `payment_attempts` insert so `escrow_details` upsert can resolve `payment_attempt_id`
/// (unique key for `escrow_details`; one audit row per attempt).
async fn persist_escrow_audit_if_applicable_verify(
    db: &Pr402Db,
    verify_request: &pr402::proto::VerifyRequest,
    correlation_id: &str,
    scheme_opt: Option<&str>,
) {
    if !payment_scheme_is_sla_escrow(scheme_opt) {
        return;
    }
    let Some(cp) = CHAIN_PROVIDER.get() else {
        warn!(
            target: LOG_SERVER_LOG,
            "escrow audit after verify skipped: CHAIN_PROVIDER not initialized"
        );
        return;
    };
    pr402::scheme::v2_solana_escrow::persist_escrow_audit_after_verify(
        db,
        cp.solana.as_ref(),
        verify_request,
        correlation_id,
    )
    .await;
}

async fn persist_escrow_audit_if_applicable_settle(
    db: &Pr402Db,
    settle_request: &pr402::proto::SettleRequest,
    correlation_id: &str,
    scheme_opt: Option<&str>,
    fund_signature: Option<&str>,
) {
    if !payment_scheme_is_sla_escrow(scheme_opt) {
        return;
    }
    let Some(cp) = CHAIN_PROVIDER.get() else {
        warn!(
            target: LOG_SERVER_LOG,
            "escrow audit after settle skipped: CHAIN_PROVIDER not initialized"
        );
        return;
    };
    pr402::scheme::v2_solana_escrow::persist_escrow_audit_after_settle(
        db,
        cp.solana.as_ref(),
        settle_request,
        correlation_id,
        fund_signature,
    )
    .await;
}

/// Optional DB: unset `DATABASE_URL` → `None`. Pool construction only — same as signer-payer
/// `signer-payer-serverless-copy/signer-payer/src/database.rs` `Database::new` (no extra probe).
fn init_pr402_db_from_env() -> Option<Pr402Db> {
    match Pr402Db::from_env_var("DATABASE_URL") {
        None => None,
        Some(Err(e)) => {
            warn!(
                target: LOG_SERVER_LOG,
                error = %e,
                "DATABASE_URL is set but pr402 could not create the Postgres pool"
            );
            None
        }
        Some(Ok(db)) => Some(db),
    }
}

/// Baseline: `signer-payer-serverless-copy/signer-payer/src/init.rs` (`LogTracer` + compact fmt).
///
/// **Vercel:** `EnvFilter::try_from_default_env()` succeeds when `RUST_LOG` is **unset or empty**, but
/// the resulting filter often defaults to **ERROR-only**, so `info!(target: "server_log", …)` never
/// prints — the dashboard “Messages” column looks empty. We treat empty/missing `RUST_LOG` like
/// signer-payer’s fallback (`server_log=info`). When `RUST_LOG` is non-empty, we **merge**
/// `server_log=info` so `target: "server_log"` is not dropped (e.g. `RUST_LOG=info` alone does not
/// match that custom target).
///
/// **Stdout + [`LineWriter`]:** Vercel’s log sink is line-oriented; unbuffered/newline-flushed stdout
/// improves capture vs default stderr buffering on `provided.al2023`.
fn init_tracing() {
    if let Err(e) = LogTracer::init() {
        eprintln!(
            "Failed to initialize LogTracer: {}. Continuing without log bridging.",
            e
        );
    }

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_level(true)
        .with_span_events(FmtSpan::NONE)
        .compact()
        .with_writer(|| LineWriter::new(std::io::stdout()));

    let rust_log = std::env::var("RUST_LOG").unwrap_or_default();
    let env_filter = if rust_log.trim().is_empty() {
        tracing_subscriber::EnvFilter::new("server_log=info")
    } else {
        tracing_subscriber::EnvFilter::from_str(rust_log.trim()).unwrap_or_else(|e| {
            eprintln!("Invalid RUST_LOG (using server_log=info): {e}");
            tracing_subscriber::EnvFilter::new("server_log=info")
        })
    };
    let server_log_directive = "server_log=info"
        .parse()
        .expect("static EnvFilter directive");
    let env_filter = env_filter.add_directive(server_log_directive);

    if let Err(e) = tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .try_init()
    {
        eprintln!(
            "Failed to initialize tracing subscriber: {}. Logs may be limited.",
            e
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();
    info!(
        target: LOG_SERVER_LOG,
        "pr402 facilitator process started (tracing initialized)"
    );

    let db = init_pr402_db_from_env();
    if PR402_DB.set(db.clone()).is_err() {
        return Err("PR402_DB: OnceLock already initialized".into());
    }

    pr402::parameters::refresh_parameters_from_db(db.as_ref()).await;
    if pr402::parameters::resolve_allowed_payment_mints(db.as_ref())
        .await
        .is_empty()
    {
        warn!(
            target: LOG_SERVER_LOG,
            "PR402_ALLOWED_PAYMENT_MINTS is not set (environment or `parameters` table). No mint allowlist is active: the facilitator accepts any SPL mint until an allowlist is configured. Configure an allowlist before production use."
        );
    }

    let facilitator_ready: Result<DynFacilitator, String> = (async {
        let config = Config::from_env().map_err(|e| format!("Config error: {}", e))?;
        let chain_provider = ChainProvider::from_config(&config)
            .await
            .map_err(|e| format!("Chain provider error: {}", e))?;
        let chain_arc = Arc::new(chain_provider);
        let facilitator = pr402::facilitator::FacilitatorLocal::new((*chain_arc).clone(), db)
            .map_err(|e| format!("Facilitator error: {}", e))?;
        if CHAIN_PROVIDER.set(chain_arc).is_err() {
            return Err("CHAIN_PROVIDER: OnceLock already initialized".into());
        }
        Ok(Arc::new(facilitator) as DynFacilitator)
    })
    .await;

    let handler = move |req: Request| {
        let facilitator_result = facilitator_ready.clone();

        Box::pin(async move {
            let path = req.uri().path().to_string();
            let method = req.method().clone();
            let query = req.uri().query().unwrap_or_default().to_string();
            let correlation_hdr: Option<String> = req
                .headers()
                .get("X-Correlation-ID")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let authorization_hdr: Option<String> = req
                .headers()
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);

            let accept = req
                .headers()
                .get("Accept")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let body = req.into_body();

            // Get facilitator instance
            let facilitator = match facilitator_result {
                Ok(f) => f.clone(),
                Err(e) => {
                    return Ok(with_api_version_v1(
                        facilitator_response!()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .header("Content-Type", "application/json")
                            .body(Body::Text(serde_json::json!({ "error": e }).to_string()))
                            .unwrap(),
                        correlation_hdr.as_deref(),
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
                ("GET", "/api/v1/facilitator/upgrade") => {
                    // Browsers and health checks use GET; Vercel only forwarded POST/OPTIONS before,
                    // so GET never reached this binary and Vercel returned 404.
                    facilitator_response!()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(Body::Text(
                            serde_json::json!({
                                "path": "/api/v1/facilitator/upgrade",
                                "method": "POST",
                                "summary": "Upgrade an X402 PaymentRequired between protocol or scheme versions.",
                                "request_body": "application/json — PaymentRequired",
                                "openapi": "/openapi.json",
                            })
                            .to_string(),
                        ))
                        .unwrap()
                }
                ("POST", "/api/v1/facilitator/upgrade") => {
                    handle_upgrade(facilitator.clone(), body).await
                }
                ("GET", "/api/v1/facilitator/supported") => {
                    handle_supported(facilitator.clone()).await
                }
                ("GET", "/api/v1/facilitator/health") => handle_health().await,
                ("GET", "/api/v1/facilitator/capabilities") => {
                    handle_capabilities(facilitator.clone()).await
                }
                ("GET", "/") => {
                    if accept.contains("text/html") {
                        Response::builder()
                            .header("Content-Type", "text/html")
                            .body(Body::Text(
                                include_str!("../../public/index.html").to_string(),
                            ))
                            .unwrap()
                    } else {
                        handle_capabilities(facilitator.clone()).await
                    }
                }
                ("GET", "/wallet.js") => {
                    let path =
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("public/wallet.js");
                    match std::fs::read_to_string(&path) {
                        Ok(js) => Response::builder()
                            .header("Content-Type", "application/javascript; charset=utf-8")
                            .header("Cache-Control", "public, max-age=3600")
                            .body(Body::Text(js))
                            .unwrap(),
                        Err(_) => error_response(
                            StatusCode::NOT_FOUND,
                            "wallet.js missing — run: cd wallet-adapter && npm ci && npm run build",
                        ),
                    }
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
                ("GET", "/api/v1/facilitator/onboard/build-tx") => {
                    handle_onboard_build_tx(facilitator.clone(), &query).await
                }
                ("GET", "/api/v1/facilitator/vault-snapshot") => {
                    handle_vault_snapshot(&query).await
                }
                ("GET", "/api/v1/facilitator/discovery") => {
                    handle_discovery(facilitator.clone(), &query).await
                }
                ("POST", "/api/v1/facilitator/build-exact-payment-tx") => {
                    if let Some(limited) = check_build_rate_limit() {
                        limited
                    } else {
                        handle_build_exact_payment_tx(body).await
                    }
                }
                ("POST", "/api/v1/facilitator/build-sla-escrow-payment-tx") => {
                    if let Some(limited) = check_build_rate_limit() {
                        limited
                    } else {
                        handle_build_sla_escrow_payment_tx(body).await
                    }
                }
                ("POST", "/api/v1/facilitator/sweep") => {
                    handle_sweep(body, authorization_hdr.as_deref()).await
                }
                ("GET", "/api/v1/facilitator/sweep-cron") => {
                    handle_sweep_cron(authorization_hdr.as_deref()).await
                }
                _ => {
                    warn!(
                        target: LOG_SERVER_LOG,
                        method = %method,
                        path = %path,
                        "no route matched (404)"
                    );
                    facilitator_response!()
                        .status(StatusCode::NOT_FOUND)
                        .header("Content-Type", "application/json")
                        .body(Body::Text(r#"{"error":"Not found"}"#.to_string()))
                        .unwrap()
                }
            };
            let status = response.status();
            let code = status.as_u16();
            if status.is_server_error() {
                error!(
                    target: LOG_SERVER_LOG,
                    method = %method,
                    path = %path,
                    status = code,
                    correlation_id = correlation_hdr.as_deref(),
                    "request completed"
                );
            } else if status.is_client_error() {
                warn!(
                    target: LOG_SERVER_LOG,
                    method = %method,
                    path = %path,
                    status = code,
                    correlation_id = correlation_hdr.as_deref(),
                    "request completed"
                );
            } else {
                info!(
                    target: LOG_SERVER_LOG,
                    method = %method,
                    path = %path,
                    status = code,
                    correlation_id = correlation_hdr.as_deref(),
                    "request completed"
                );
            }
            Ok(with_api_version_v1(response, correlation_hdr.as_deref()))
        })
    };

    run(handler).await
}

mod handlers;
use handlers::*;

fn query_param(query: &str, key: &str) -> String {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

fn error_response(status: StatusCode, message: &str) -> Response<Body> {
    error_response_with_optional_correlation(status, message, None, None)
}

fn error_response_with_optional_correlation(
    status: StatusCode,
    message: &str,
    code: Option<&str>,
    correlation_id: Option<&str>,
) -> Response<Body> {
    const MAX_MSG_LOG: usize = 2048;
    let log_msg: String = message.chars().take(MAX_MSG_LOG).collect();

    // Machine code defaults to status name if not provided
    let machine_code = code
        .unwrap_or_else(|| status.canonical_reason().unwrap_or("INTERNAL_ERROR"))
        .to_uppercase()
        .replace(" ", "_");

    if status.is_server_error() {
        error!(
            target: LOG_SERVER_LOG,
            status = %status.as_u16(),
            code = %machine_code,
            correlation_id = ?correlation_id,
            message = %log_msg,
            "facilitator JSON error response"
        );
    } else {
        warn!(
            target: LOG_SERVER_LOG,
            status = %status.as_u16(),
            code = %machine_code,
            correlation_id = ?correlation_id,
            message = %log_msg,
            "facilitator JSON error response"
        );
    }

    // Canonical error response fields: `error` (human-readable), `code` (machine-readable).
    let json = serde_json::json!({
        "error": message,
        "code": machine_code,
        "correlationId": correlation_id
    });

    let mut res = facilitator_response!()
        .status(status)
        .header("Content-Type", "application/json");

    if let Some(cid) = correlation_id {
        res = res.header("X-Correlation-ID", cid);
    }

    res.body(Body::Text(json.to_string())).unwrap()
}
