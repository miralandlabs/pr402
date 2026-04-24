//! Global `parameters` cache (DB-backed), following signer-payer `PARAMETERS` + `init_parameters` pattern.
//! Values in Postgres avoid Vercel env size limits and can change without redeploy (TTL refresh).

use crate::db::Pr402Db;
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Once, OnceLock, RwLock};
use std::time::{Duration, Instant};
use tracing::warn;

/// In-memory cache entry (shared across async tasks via std mutex; hold time is tiny).
pub struct ParametersCache {
    map: HashMap<String, String>,
    /// Last attempt to load from Postgres (success or failure) — backs off hammering DB.
    last_fetch: Option<Instant>,
}

impl ParametersCache {
    fn empty() -> Self {
        Self {
            map: HashMap::new(),
            last_fetch: None,
        }
    }
}

/// Global parameters cache (cf. signer-payer `PARAMETERS` + `init_parameters`).
pub static PARAMETERS: OnceLock<RwLock<ParametersCache>> = OnceLock::new();

fn cache_store() -> &'static RwLock<ParametersCache> {
    PARAMETERS.get_or_init(|| RwLock::new(ParametersCache::empty()))
}

/// How often to re-query Postgres (per serverless instance). **Env `PR402_PARAMETERS_CACHE_TTL_SEC` only** —
/// not loaded from the `parameters` table (a row with that name would be ignored for TTL).
pub fn parameters_cache_ttl() -> Duration {
    static TTL: OnceLock<Duration> = OnceLock::new();
    *TTL.get_or_init(|| {
        std::env::var("PR402_PARAMETERS_CACHE_TTL_SEC")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(60))
    })
}

fn cache_needs_refresh(cache: &ParametersCache, ttl: Duration) -> bool {
    match cache.last_fetch {
        None => true,
        Some(t) => t.elapsed() > ttl,
    }
}

/// Refresh cache from DB when stale or empty.
pub async fn refresh_parameters_from_db(db: Option<&Pr402Db>) {
    let Some(db) = db else {
        return;
    };
    let ttl = parameters_cache_ttl();
    {
        let r = cache_store().read().ok();
        if let Some(c) = r {
            if !cache_needs_refresh(&c, ttl) {
                return;
            }
        }
    }

    let now = Instant::now();
    match db.fetch_parameters_map().await {
        Ok(map) => {
            if let Ok(mut w) = cache_store().write() {
                w.map = map;
                w.last_fetch = Some(now);
            }
        }
        Err(e) => {
            warn!(error = %e, "parameters table read failed (run migrations/init.sql?)");
            if let Ok(mut w) = cache_store().write() {
                w.last_fetch = Some(now);
            }
        }
    }
}

/// `param_name` keys (also used as env var names where noted).
pub const PR402_ONBOARD_HMAC_SECRET: &str = "PR402_ONBOARD_HMAC_SECRET";
pub const PR402_ONBOARD_CHALLENGE_TTL_SEC: &str = "PR402_ONBOARD_CHALLENGE_TTL_SEC";
/// Comma-separated SPL mint pubkeys (or single mint) for quote / payment rails — optional consumer.
pub const PR402_QUOTE_SPL_MINTS: &str = "PR402_QUOTE_SPL_MINTS";
/// Comma-separated SPL mint pubkeys (plus `11111111111111111111111111111111` for Native SOL) allowed for payment.
/// Effectively the "Sovereign Whitelist" for the facilitator.
pub const PR402_ALLOWED_PAYMENT_MINTS: &str = "PR402_ALLOWED_PAYMENT_MINTS";

/// Maximum number of new SplitVaults the facilitator will pay to create per day (anti-spam).
pub const PR402_MAX_DAILY_PROVISION_COUNT: &str = "PR402_MAX_DAILY_PROVISION_COUNT";

/// Min spendable lamports in UniversalSettle SOL storage before submitting a Sweep (gas not worth dust).
pub const PR402_SWEEP_MIN_SPENDABLE_LAMPORTS: &str = "PR402_SWEEP_MIN_SPENDABLE_LAMPORTS";
/// Default min SPL raw amount when mint has no entry in [`PR402_SWEEP_MIN_SPL_RAW_BY_MINT`] (see [`DEFAULT_SWEEP_MIN_SPL_RAW_DEFAULT`]).
pub const PR402_SWEEP_MIN_SPL_RAW_DEFAULT: &str = "PR402_SWEEP_MIN_SPL_RAW_DEFAULT";
/// JSON object: `{ "<mint_base58>": "<raw_u64>", ... }` for per-token sweep floors.
pub const PR402_SWEEP_MIN_SPL_RAW_BY_MINT: &str = "PR402_SWEEP_MIN_SPL_RAW_BY_MINT";
/// Bearer token for authenticated cron-driven sweep execution endpoint.
pub const PR402_SWEEP_CRON_TOKEN: &str = "PR402_SWEEP_CRON_TOKEN";
/// Minimum seconds between sweep attempts per provider rail when cron sweeper runs.
pub const PR402_SWEEP_CRON_COOLDOWN_SEC: &str = "PR402_SWEEP_CRON_COOLDOWN_SEC";
/// Only consider providers with successful settles within this recent window (seconds).
pub const PR402_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC: &str =
    "PR402_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC";
/// Max provider rails processed per cron sweep run.
pub const PR402_SWEEP_CRON_BATCH_LIMIT: &str = "PR402_SWEEP_CRON_BATCH_LIMIT";

/// Fallback when `PR402_SWEEP_MIN_SPENDABLE_LAMPORTS` is unset (0.03 SOL).
pub const DEFAULT_SWEEP_MIN_SPENDABLE_LAMPORTS: u64 = 30_000_000;

/// Fallback when `PR402_SWEEP_MIN_SPL_RAW_DEFAULT` is unset (~$3 at 6 decimals; typical stablecoin rails).
pub const DEFAULT_SWEEP_MIN_SPL_RAW_DEFAULT: u64 = 3_000_000;

/// Fallback when `PR402_MAX_DAILY_PROVISION_COUNT` is unset.
pub const DEFAULT_MAX_DAILY_PROVISION_COUNT: u64 = 50;
pub const DEFAULT_SWEEP_CRON_COOLDOWN_SEC: u64 = 300;
pub const DEFAULT_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC: u64 = 86_400;
pub const DEFAULT_SWEEP_CRON_BATCH_LIMIT: u64 = 50;

/// Read cache then env (no async DB fetch). Call [`refresh_parameters_from_db`] before settle so cache is warm.
pub fn resolve_string_sync(param_key: &str, env_key: &str) -> Option<String> {
    let from_cache = cache_store()
        .read()
        .ok()
        .and_then(|c| c.map.get(param_key).cloned())
        .filter(|s| !s.is_empty());
    if from_cache.is_some() {
        return from_cache;
    }
    std::env::var(env_key).ok().filter(|s| !s.is_empty())
}

/// See [`resolve_string_sync`].
pub fn resolve_u64_sync(param_key: &str, env_key: &str, default: u64) -> u64 {
    resolve_string_sync(param_key, env_key)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(default)
}

/// Effective SPL sweep threshold for `mint` (per-mint JSON overrides default).
pub fn resolve_sweep_min_spl_raw_for_mint(mint: &Pubkey) -> u64 {
    let default = resolve_u64_sync(
        PR402_SWEEP_MIN_SPL_RAW_DEFAULT,
        PR402_SWEEP_MIN_SPL_RAW_DEFAULT,
        DEFAULT_SWEEP_MIN_SPL_RAW_DEFAULT,
    );
    let Some(raw) = resolve_string_sync(
        PR402_SWEEP_MIN_SPL_RAW_BY_MINT,
        PR402_SWEEP_MIN_SPL_RAW_BY_MINT,
    ) else {
        return default;
    };
    let map: HashMap<String, String> = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            warn!(error = %e, "PR402_SWEEP_MIN_SPL_RAW_BY_MINT invalid JSON");
            return default;
        }
    };
    map.get(&mint.to_string())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(default)
}

/// Resolve string: non-empty **database** value wins (dynamic updates), else **environment**, else `None`.
pub async fn resolve_string(
    db: Option<&Pr402Db>,
    param_key: &str,
    env_var: Option<&str>,
) -> Option<String> {
    if db.is_some() {
        refresh_parameters_from_db(db).await;
    }

    let from_db = cache_store()
        .read()
        .ok()
        .and_then(|c| c.map.get(param_key).cloned())
        .filter(|s| !s.is_empty());

    if from_db.is_some() {
        return from_db;
    }

    env_var
        .and_then(|name| std::env::var(name).ok())
        .filter(|s| !s.is_empty())
}

pub async fn resolve_onboard_hmac_secret(db: Option<&Pr402Db>) -> Option<String> {
    resolve_string(
        db,
        PR402_ONBOARD_HMAC_SECRET,
        Some(PR402_ONBOARD_HMAC_SECRET),
    )
    .await
}

pub async fn resolve_onboard_challenge_ttl_sec(db: Option<&Pr402Db>, default: u64) -> u64 {
    let s = resolve_string(
        db,
        PR402_ONBOARD_CHALLENGE_TTL_SEC,
        Some(PR402_ONBOARD_CHALLENGE_TTL_SEC),
    )
    .await;
    if let Some(ref raw) = s {
        if let Ok(v) = raw.parse::<u64>() {
            return v;
        }
    }
    default
}

/// Quote mints from `PR402_QUOTE_SPL_MINTS` (commas / ASCII whitespace). DB or env via [`resolve_string`].
pub async fn resolve_quote_spl_mints(db: Option<&Pr402Db>) -> Vec<String> {
    let Some(raw) = resolve_string(db, PR402_QUOTE_SPL_MINTS, Some(PR402_QUOTE_SPL_MINTS)).await
    else {
        return Vec::new();
    };
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
/// Allowed mints from `PR402_ALLOWED_PAYMENT_MINTS`. DB or env via [`resolve_string`].
pub async fn resolve_allowed_payment_mints(db: Option<&Pr402Db>) -> Vec<String> {
    let Some(raw) = resolve_string(
        db,
        PR402_ALLOWED_PAYMENT_MINTS,
        Some(PR402_ALLOWED_PAYMENT_MINTS),
    )
    .await
    else {
        return Vec::new();
    };
    raw.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

static PAYMENT_MINT_ALLOWLIST_PERMISSIVE_WARNED: Once = Once::new();

fn warn_payment_mint_allowlist_permissive_once() {
    PAYMENT_MINT_ALLOWLIST_PERMISSIVE_WARNED.call_once(|| {
        warn!(
            target: "server_log",
            "PR402_ALLOWED_PAYMENT_MINTS is unset or empty — all payment mints are accepted (permissive). Set a comma-separated list in env or the `parameters` table (same key) before production."
        );
    });
}

/// Enforce [`PR402_ALLOWED_PAYMENT_MINTS`] when non-empty (DB via [`resolve_string`], then env).
/// Empty / missing → permissive (all mints); logs once per process in that mode.
///
/// Returns the same user-visible message as verify/settle `InvalidPayload` for disallowed mints.
pub async fn ensure_allowed_payment_mint(
    db: Option<&Pr402Db>,
    mint: &Pubkey,
) -> Result<(), String> {
    let allowed = resolve_allowed_payment_mints(db).await;
    if allowed.is_empty() {
        warn_payment_mint_allowlist_permissive_once();
        return Ok(());
    }

    let mint_str = mint.to_string();
    if allowed.iter().any(|m| m == &mint_str) {
        return Ok(());
    }

    warn!(
        target: "server_log",
        mint = %mint_str,
        "payment mint not in PR402_ALLOWED_PAYMENT_MINTS"
    );
    Err(format!(
        "Mint {} is not supported for payment by this facilitator. Approved assets: {}.",
        mint_str,
        allowed.join(", ")
    ))
}

/// Stage A (non-blocking): when an allowlist is configured, warn if any `accepts[].asset` is absent from it.
pub async fn warn_accepts_assets_not_in_allowlist(
    db: Option<&Pr402Db>,
    payment_required: &crate::proto::PaymentRequired,
) {
    let allowed = resolve_allowed_payment_mints(db).await;
    if allowed.is_empty() {
        return;
    }
    let crate::proto::PaymentRequired::V2(body) = payment_required;
    for (i, accept) in body.accepts.iter().enumerate() {
        let Some(asset_str) = accept.get("asset").and_then(|v| v.as_str()) else {
            continue;
        };
        let Ok(pk) = Pubkey::from_str(asset_str) else {
            continue;
        };
        let mint_str = pk.to_string();
        if !allowed.iter().any(|m| m == &mint_str) {
            warn!(
                target: "server_log",
                accepts_index = i,
                asset = %mint_str,
                "accepts[] asset is not listed in PR402_ALLOWED_PAYMENT_MINTS — verify, settle, and build-* endpoints will reject this rail for buyers"
            );
        }
    }
}

pub async fn resolve_sweep_cron_token(db: Option<&Pr402Db>) -> Option<String> {
    resolve_string(db, PR402_SWEEP_CRON_TOKEN, Some(PR402_SWEEP_CRON_TOKEN)).await
}

pub async fn resolve_sweep_cron_cooldown_sec(db: Option<&Pr402Db>, default: u64) -> u64 {
    resolve_string(
        db,
        PR402_SWEEP_CRON_COOLDOWN_SEC,
        Some(PR402_SWEEP_CRON_COOLDOWN_SEC),
    )
    .await
    .and_then(|s| s.parse::<u64>().ok())
    .unwrap_or(default)
}

pub async fn resolve_sweep_cron_recent_settle_window_sec(
    db: Option<&Pr402Db>,
    default: u64,
) -> u64 {
    resolve_string(
        db,
        PR402_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC,
        Some(PR402_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC),
    )
    .await
    .and_then(|s| s.parse::<u64>().ok())
    .unwrap_or(default)
}

pub async fn resolve_sweep_cron_batch_limit(db: Option<&Pr402Db>, default: u64) -> u64 {
    resolve_string(
        db,
        PR402_SWEEP_CRON_BATCH_LIMIT,
        Some(PR402_SWEEP_CRON_BATCH_LIMIT),
    )
    .await
    .and_then(|s| s.parse::<u64>().ok())
    .unwrap_or(default)
}
