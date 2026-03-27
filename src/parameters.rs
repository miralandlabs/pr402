//! Global `parameters` cache (DB-backed), following signer-payer `PARAMETERS` + `init_parameters` pattern.
//! Values in Postgres avoid Vercel env size limits and can change without redeploy (TTL refresh).

use crate::db::Pr402Db;
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
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

/// Min spendable lamports in UniversalSettle SOL storage before submitting a Sweep (gas not worth dust).
pub const PR402_SWEEP_MIN_SPENDABLE_LAMPORTS: &str = "PR402_SWEEP_MIN_SPENDABLE_LAMPORTS";
/// Default min SPL raw amount when mint has no entry in [`PR402_SWEEP_MIN_SPL_RAW_BY_MINT`] (see [`DEFAULT_SWEEP_MIN_SPL_RAW_DEFAULT`]).
pub const PR402_SWEEP_MIN_SPL_RAW_DEFAULT: &str = "PR402_SWEEP_MIN_SPL_RAW_DEFAULT";
/// JSON object: `{ "<mint_base58>": "<raw_u64>", ... }` for per-token sweep floors.
pub const PR402_SWEEP_MIN_SPL_RAW_BY_MINT: &str = "PR402_SWEEP_MIN_SPL_RAW_BY_MINT";

/// Fallback when `PR402_SWEEP_MIN_SPENDABLE_LAMPORTS` is unset (0.03 SOL).
pub const DEFAULT_SWEEP_MIN_SPENDABLE_LAMPORTS: u64 = 30_000_000;

/// Fallback when `PR402_SWEEP_MIN_SPL_RAW_DEFAULT` is unset (~$3 at 6 decimals; typical stablecoin rails).
pub const DEFAULT_SWEEP_MIN_SPL_RAW_DEFAULT: u64 = 3_000_000;

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
