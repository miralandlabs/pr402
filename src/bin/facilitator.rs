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
const SCHEMA_VERSION: &str = "1.1.0";

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
struct SettlementKeeperHealth {
    vault_sweep_cron_configured: bool,
    sla_escrow_settle_cron_configured: bool,
    database_connected: bool,
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
    /// HTTP RPC URL for browser `Connection` (e.g. `sendRawTransaction`). Same as facilitator `SOLANA_RPC_URL` when not localhost; otherwise the public cluster URL for `solanaNetwork`.
    solana_wallet_rpc_url: String,
    settlement_keeper: SettlementKeeperHealth,
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
    /// Reference metadata for the SLA-Escrow oracle profiles (multi-category).
    /// Plural array — one entry per advertised profile (api-quality / onchain-transfer / file-delivery).
    #[serde(skip_serializing_if = "Option::is_none")]
    sla_escrow_oracle_profiles: Option<Vec<SlaEscrowOracleProfileInfo>>,
    /// Static seller endpoint decision matrix (`public/seller-endpoint-guide.json`).
    #[serde(skip_serializing_if = "Option::is_none")]
    seller_endpoint_guide: Option<serde_json::Value>,
}

/// Published pointers for one SLA-Escrow oracle profile (one entry of `slaEscrowOracleProfiles[]`).
/// Generic across delivery categories (api-quality, onchain-transfer, file-delivery, future).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SlaEscrowOracleProfileInfo {
    /// Canonical profile id, e.g. `x402/oracles/api-quality/v1`.
    profile_id: String,
    /// URL of the normative spec (NORMATIVE.md) for this profile.
    normative_spec_url: String,
    /// Repository path (relative to the linked repo root) where the profile lives.
    /// Empty string if the deployment doesn't want to advertise a specific path.
    #[serde(skip_serializing_if = "String::is_empty")]
    repository_path: String,
    /// Optional advertised default oracle authority pubkey for this profile.
    /// Buyers may use this when the seller's `accepts[].extra.oracleProfiles[]` does
    /// not list one. **Buyers MUST still confirm trust.**
    #[serde(skip_serializing_if = "Option::is_none")]
    default_operator_pubkey: Option<String>,
    /// Optional registry URL hint for the seller's HMAC-bound upload flow
    /// (`POST /v1/registry/sla|delivery|blob`).
    #[serde(skip_serializing_if = "Option::is_none")]
    registry_url: Option<String>,
    /// Free-form note for integrators (e.g. evidence registry policy).
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence_registry_note: Option<String>,
    /// Wave A §3.2 — when pr402 has probed this oracle's `/health` and the
    /// probe failed, this field is `Some(true)` so buyers can skip the
    /// profile. When `None`, the gate is either disabled (default) or no
    /// `registry_url` is configured for probing. Health probes happen out of
    /// band; the cache lifetime is short (30s) so a transient failure cannot
    /// keep a profile dark for long.
    #[serde(skip_serializing_if = "Option::is_none")]
    unhealthy: Option<bool>,
    /// Last-probe error string when `unhealthy = Some(true)`. Surfaced for
    /// operator diagnostics; agents typically ignore this and fall back to
    /// another profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_health_error: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesFeatures {
    universal_settle_exact: bool,
    sla_escrow: bool,
    unsigned_exact_payment_tx_build: bool,
    unsigned_sla_escrow_payment_tx_build: bool,
    /// True when `GET /sellers/{wallet}/preview` returns the `lifecycle` block (preview → activate → verify).
    /// Agents can use this to decide whether to trust `nextStep` or fall back to parsing
    /// `schemes["exact"].status` + separately probing the registry.
    seller_lifecycle_block: bool,
    /// True when `POST /sellers/{wallet}/register` accepts both base58 and base64 signature encodings.
    /// Browser wallet adapters return base64 natively — this flag lets clients skip the
    /// in-page base58 encoder when the server already accepts their raw output.
    accepts_base64_onboard_signature: bool,
    /// True when build responses include `signerPubkeys[]`. Agents can use this to map
    /// each `signatures[i]` slot to its required pubkey without re-decoding the tx.
    build_response_signer_pubkeys: bool,
    /// True when `GET /providers` + `GET /providers/{wallet}` return the public seller
    /// directory. Absent on deployments without a database.
    public_provider_directory: bool,
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
    build_oracle_confirm_tx: HttpEndpointInfo,
    /// `POST` — build unsigned refund transaction (merchant → payer `TransferChecked`).
    build_refund_tx: HttpEndpointInfo,
    sweep: HttpEndpointInfo,
    sweep_cron: HttpEndpointInfo,
    /// `POST` — drive permissionless `ReleasePayment` / `RefundPayment` for funded
    /// sla-escrow payments whose oracle has rendered a verdict or whose TTL has expired.
    /// Mirrors the existing `sweep` shape for the sla-escrow rail. Bearer-authenticated.
    sla_escrow_settle: HttpEndpointInfo,
    /// `GET` — Vercel cron entry point for the sla-escrow settlement loop. Same auth
    /// + behavior as `sla_escrow_settle` with default request parameters.
    sla_escrow_settle_cron: HttpEndpointInfo,
    /// `GET` — read-only multi-rail preview + lifecycle ladder (`/sellers/{wallet}/preview`).
    seller_preview: HttpEndpointInfo,
    /// `GET` — HMAC challenge before registry submit (`/sellers/{wallet}/challenge`).
    seller_challenge: HttpEndpointInfo,
    /// `POST` — wallet-signed registry submit (`/sellers/{wallet}/register`).
    seller_register: HttpEndpointInfo,
    /// `POST` — on-chain activate: unsigned CreateVault tx (`/sellers/provision-tx`).
    seller_provision_tx: HttpEndpointInfo,
    /// `POST` — off-chain retirement (`/sellers/{wallet}/retire`).
    seller_retire: HttpEndpointInfo,
    /// `GET` — public seller directory (paged).
    providers: HttpEndpointInfo,
    /// `GET` — public seller directory single-wallet lookup. `{wallet}` is a path segment.
    provider: HttpEndpointInfo,
    /// `POST` — wallet-authenticated seller payment history (same HMAC flow as onboard).
    seller_payments: HttpEndpointInfo,
    supported: HttpEndpointInfo,
    health: HttpEndpointInfo,
    capabilities: HttpEndpointInfo,
    /// `GET` — single-rail SchemeOnboardInfo (`/sellers/{wallet}/rails/{scheme}`).
    seller_rail_info: HttpEndpointInfo,
    /// `POST` — enrich naive PaymentRequired for HTTP 402 (`/payment-required/enrich`).
    payment_required_enrich: HttpEndpointInfo,
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

fn seller_endpoint_guide_json() -> Option<serde_json::Value> {
    serde_json::from_str(include_str!("../../public/seller-endpoint-guide.json")).ok()
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
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
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
    });
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
        // Hard-gate the startup when the operator set `PR402_REQUIRE_MINT_ALLOWLIST=true`.
        // This prevents the silent "permissive mode" trap where a production deployment
        // forgets to configure an allowlist and silently accepts any SPL mint from buyers.
        // The gate reads from the same DB `parameters` table the allowlist itself uses,
        // so operators can toggle it without redeploying.
        if pr402::parameters::resolve_require_mint_allowlist(db.as_ref()).await {
            error!(
                target: LOG_SERVER_LOG,
                "PR402_REQUIRE_MINT_ALLOWLIST is enabled but PR402_ALLOWED_PAYMENT_MINTS is empty. Refusing to start. Configure an allowlist (environment or `parameters` table) or disable the gate explicitly."
            );
            return Err(
                "Refusing to start: PR402_REQUIRE_MINT_ALLOWLIST=true with empty PR402_ALLOWED_PAYMENT_MINTS"
                    .into(),
            );
        }
        warn!(
            target: LOG_SERVER_LOG,
            "PR402_ALLOWED_PAYMENT_MINTS is not set (environment or `parameters` table). No mint allowlist is active: the facilitator accepts any SPL mint until an allowlist is configured. Configure an allowlist before production use, or set PR402_REQUIRE_MINT_ALLOWLIST=true to refuse start when unconfigured."
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

            let effective_method = if method.as_str() == "HEAD" {
                "GET"
            } else {
                method.as_str()
            };
            let response = match (effective_method, path.as_str()) {
                ("OPTIONS", p) if p.starts_with("/api/v1/facilitator") => cors_preflight_response(),
                ("POST", "/api/v1/facilitator/verify") => {
                    handle_verify(facilitator.clone(), body, correlation_hdr.as_deref()).await
                }
                ("POST", "/api/v1/facilitator/settle") => {
                    handle_settle(facilitator.clone(), body, correlation_hdr.as_deref()).await
                }
                ("GET", seller_api::PAYMENT_REQUIRED_ENRICH) => {
                    facilitator_response!()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(Body::Text(
                            serde_json::json!({
                                "path": seller_api::PAYMENT_REQUIRED_ENRICH,
                                "method": "POST",
                                "summary": "Enrich a naive PaymentRequired for HTTP 402 (vault PDA + extra metadata).",
                                "request_body": "application/json — PaymentRequired",
                                "openapi": "/openapi.json",
                            })
                            .to_string(),
                        ))
                        .unwrap()
                }
                ("POST", seller_api::PAYMENT_REQUIRED_ENRICH) => {
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
                    if accept.contains("text/markdown") {
                        Response::builder()
                            .header("Content-Type", "text/markdown; charset=utf-8")
                            .header("Link", "</openapi.json>; rel=\"service-desc\"")
                            .body(Body::Text(
                                include_str!("../../public/agent-integration.md").to_string(),
                            ))
                            .unwrap()
                    } else if accept.contains("text/html") {
                        Response::builder()
                            .header("Content-Type", "text/html")
                            .header("Link", "</openapi.json>; rel=\"service-desc\"")
                            .body(Body::Text(
                                include_str!("../../public/index.html").to_string(),
                            ))
                            .unwrap()
                    } else {
                        let mut res = handle_capabilities(facilitator.clone()).await;
                        res.headers_mut().insert(
                            http::header::LINK,
                            http::HeaderValue::from_static("</openapi.json>; rel=\"service-desc\""),
                        );
                        res
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
                // Site icon — served at both the canonical `/pr402.png` (referenced by
                // `<link rel="icon">` in `public/index.html`) and `/favicon.ico` (browser
                // default probe when a JSON endpoint like `/capabilities` is shown without
                // any `<link>` tags). `apple-touch-icon*.png` is for iOS home-screen bookmarks.
                // The PNG is compiled in via `include_bytes!` so it ships with the binary
                // alongside the already-embedded `public/index.html`.
                ("GET", "/pr402.png")
                | ("GET", "/favicon.ico")
                | ("GET", "/apple-touch-icon.png")
                | ("GET", "/apple-touch-icon-precomposed.png") => {
                    const ICON_BYTES: &[u8] = include_bytes!("../../public/pr402.png");
                    Response::builder()
                        .header("Content-Type", "image/png")
                        .header("Cache-Control", "public, max-age=86400, immutable")
                        .body(Body::Binary(ICON_BYTES.to_vec()))
                        .unwrap()
                }
                ("GET", p)
                    if p.starts_with(seller_api::SELLERS_PREFIX)
                        && p.ends_with("/preview")
                        && p.len() > seller_api::SELLERS_PREFIX.len() + "/preview".len() =>
                {
                    if let Some(w) = seller_api::parse_sellers_wallet_suffix(p, "/preview") {
                        handle_onboard_preview(
                            facilitator.clone(),
                            &seller_api::preview_query(&w),
                        )
                        .await
                    } else {
                        facilitator_response!()
                            .status(StatusCode::NOT_FOUND)
                            .header("Content-Type", "application/json")
                            .body(Body::Text(r#"{"error":"Not found"}"#.to_string()))
                            .unwrap()
                    }
                }
                ("GET", p)
                    if p.starts_with(seller_api::SELLERS_PREFIX)
                        && p.ends_with("/challenge")
                        && p.len() > seller_api::SELLERS_PREFIX.len() + "/challenge".len() =>
                {
                    if let Some(w) = seller_api::parse_sellers_wallet_suffix(p, "/challenge") {
                        handle_onboard_challenge(&seller_api::challenge_query(&w)).await
                    } else {
                        facilitator_response!()
                            .status(StatusCode::NOT_FOUND)
                            .header("Content-Type", "application/json")
                            .body(Body::Text(r#"{"error":"Not found"}"#.to_string()))
                            .unwrap()
                    }
                }
                ("POST", p)
                    if p.starts_with(seller_api::SELLERS_PREFIX)
                        && p.ends_with("/register")
                        && p.len() > seller_api::SELLERS_PREFIX.len() + "/register".len() =>
                {
                    if let Some(w) = seller_api::parse_sellers_wallet_suffix(p, "/register") {
                        handle_onboard_submit(facilitator.clone(), &w, body).await
                    } else {
                        facilitator_response!()
                            .status(StatusCode::NOT_FOUND)
                            .header("Content-Type", "application/json")
                            .body(Body::Text(r#"{"error":"Not found"}"#.to_string()))
                            .unwrap()
                    }
                }
                ("POST", p)
                    if p.starts_with(seller_api::SELLERS_PREFIX)
                        && p.ends_with("/retire")
                        && p.len() > seller_api::SELLERS_PREFIX.len() + "/retire".len() =>
                {
                    if let Some(w) = seller_api::parse_sellers_wallet_suffix(p, "/retire") {
                        handle_onboard_retire(&w, body).await
                    } else {
                        facilitator_response!()
                            .status(StatusCode::NOT_FOUND)
                            .header("Content-Type", "application/json")
                            .body(Body::Text(r#"{"error":"Not found"}"#.to_string()))
                            .unwrap()
                    }
                }
                ("GET", p)
                    if p.starts_with(seller_api::SELLERS_PREFIX) && p.contains("/rails/") =>
                {
                    if let Some((w, scheme)) = seller_api::parse_sellers_rail(p) {
                        let asset = query_param(&query, "asset");
                        let q = seller_api::discovery_query(
                            &w,
                            &scheme,
                            if asset.is_empty() {
                                None
                            } else {
                                Some(&asset)
                            },
                        );
                        handle_discovery(facilitator.clone(), &q).await
                    } else {
                        facilitator_response!()
                            .status(StatusCode::NOT_FOUND)
                            .header("Content-Type", "application/json")
                            .body(Body::Text(r#"{"error":"Not found"}"#.to_string()))
                            .unwrap()
                    }
                }
                ("POST", seller_api::PROVISION_TX) => {
                    if let Some(limited) = check_build_rate_limit() {
                        limited
                    } else {
                        handle_onboard_provision(facilitator.clone(), body).await
                    }
                }
                ("GET", "/api/v1/facilitator/providers") => {
                    handle_public_providers_list(&query).await
                }
                // Single-wallet directory lookup. The handler is given the wallet pubkey
                // as the trailing path segment; guard with `len > prefix.len()` so the
                // bare `/providers` route (handled above) still hits list semantics.
                ("GET", p)
                    if p.starts_with("/api/v1/facilitator/providers/")
                        && p.len() > "/api/v1/facilitator/providers/".len() =>
                {
                    let wallet = &p["/api/v1/facilitator/providers/".len()..];
                    handle_public_provider_single(wallet).await
                }
                ("POST", "/api/v1/facilitator/seller/payments") => {
                    handle_seller_payments_list(&query, body).await
                }
                ("GET", "/api/v1/facilitator/vault-snapshot") => {
                    handle_vault_snapshot(&query).await
                }
                ("POST", "/api/v1/facilitator/build-exact-payment-tx") => {
                    if let Some(limited) = check_build_rate_limit() {
                        limited
                    } else {
                        handle_build_exact_payment_tx(body).await
                    }
                }
                ("POST", "/api/v1/facilitator/oracle/build-confirm") => {
                    if let Some(limited) = check_build_rate_limit() {
                        limited
                    } else {
                        handlers::build::handle_build_oracle_confirm_tx(body).await
                    }
                }
                ("POST", "/api/v1/facilitator/build-sla-escrow-payment-tx") => {
                    if let Some(limited) = check_build_rate_limit() {
                        limited
                    } else {
                        handle_build_sla_escrow_payment_tx(body).await
                    }
                }
                ("POST", "/api/v1/facilitator/build-refund-tx") => {
                    if let Some(limited) = check_build_rate_limit() {
                        limited
                    } else {
                        handle_build_refund_tx(body).await
                    }
                }
                ("POST", "/api/v1/facilitator/sweep") => {
                    handle_sweep(body, authorization_hdr.as_deref()).await
                }
                ("GET", "/api/v1/facilitator/sweep-cron") => {
                    handle_sweep_cron(authorization_hdr.as_deref()).await
                }
                ("POST", "/api/v1/facilitator/sla-escrow-settle") => {
                    handle_sla_escrow_settle(body, authorization_hdr.as_deref()).await
                }
                ("GET", "/api/v1/facilitator/sla-escrow-settle-cron") => {
                    handle_sla_escrow_settle_cron(authorization_hdr.as_deref()).await
                }
                ("POST", "/api/v1/facilitator/sla-escrow-close") => {
                    handle_sla_escrow_close(body, authorization_hdr.as_deref()).await
                }
                ("GET", "/api/v1/facilitator/sla-escrow-close-cron") => {
                    handle_sla_escrow_close_cron(authorization_hdr.as_deref()).await
                }
                ("POST", "/api/v1/facilitator/build-sla-escrow-settle-tx") => {
                    handle_build_sla_escrow_settle_tx(body).await
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
use handlers::seller_api;
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
