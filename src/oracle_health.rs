//! Wave A §3.2 — opt-in oracle health gate.
//!
//! Probes each advertised oracle profile's `/health` endpoint (derived from
//! the operator's `registry_url`) before:
//!
//! 1. Including the profile in `GET /capabilities` (unhealthy profiles get an
//!    `unhealthy: true` annotation).
//! 2. Building an SLA-Escrow FundPayment that binds to an oracle authority
//!    mapped to an unhealthy profile (returns HTTP 503 `oracle_unhealthy`).
//!
//! ## Design
//!
//! - **In-process cache** keyed by health-URL with a 30-second positive TTL
//!   and a 30-second negative TTL. A single tokio runtime holds the cache;
//!   serverless cold starts pay one probe per profile.
//! - **HTTP probe**: `GET <health_url>` with a 2-second total timeout. Any
//!   non-2xx (or timeout, or transport error) marks the profile unhealthy
//!   with the underlying error string captured for diagnostics.
//! - **Default OFF**: governed by [`PR402_SLA_ESCROW_REQUIRE_ORACLE_HEALTHY`].
//!   When unset / falsy, [`probe_unhealthy`] always returns `Ok(None)` and the
//!   gate is a no-op.
//! - **Defense-in-depth, not a guarantee**: the oracle could go offline a
//!   millisecond after the probe. Combined with on-chain `expires_at` and
//!   buyer-side reclaim, the gate closes the obvious "bind to a dead oracle"
//!   failure mode without changing on-chain semantics.
//!
//! ## Health URL derivation
//!
//! The `registry_url` advertised on `/capabilities` is the seller's HMAC-bound
//! registration endpoint (e.g. `https://oracle.example.com/v1/registry`). The
//! oracle's `/health` is at the binary's root, so we trim trailing
//! `/v1/registry` (or anything starting with `/v1`) and append `/health`. When
//! `registry_url` is absent, the gate cannot probe and the profile passes.
//!
//! ## Concurrency
//!
//! [`Cache`] is a `tokio::sync::RwLock<HashMap<...>>`. Reads (the common path)
//! take a read lock; writes (after a successful or failed probe) take a write
//! lock. Probe inflight collapsing is *not* implemented — at worst, two
//! near-simultaneous capability requests cause two probes; both write the
//! same result.

use std::{
    collections::HashMap,
    sync::OnceLock,
    time::{Duration, Instant},
};

use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::parameters as p;

/// Positive cache lifetime: a profile that probed healthy stays healthy in
/// cache for this long.
const POSITIVE_TTL: Duration = Duration::from_secs(30);
/// Negative cache lifetime: a profile that probed unhealthy is *not* re-probed
/// for this long. Keeps capability/build hot paths fast even when one oracle
/// is sustained-down.
const NEGATIVE_TTL: Duration = Duration::from_secs(30);
/// Per-probe HTTP timeout. Generous enough for cold-started oracles, short
/// enough that a serverless invocation won't time out the calling function.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// One cached probe outcome for a given health URL.
#[derive(Debug, Clone)]
struct CachedHealth {
    /// `Ok(())` for a 2xx response; `Err(msg)` otherwise.
    result: Result<(), String>,
    /// Wall-clock time when the probe completed.
    cached_at: Instant,
}

impl CachedHealth {
    fn is_fresh(&self) -> bool {
        let ttl = if self.result.is_ok() {
            POSITIVE_TTL
        } else {
            NEGATIVE_TTL
        };
        self.cached_at.elapsed() < ttl
    }
}

type Cache = RwLock<HashMap<String, CachedHealth>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn shared_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .user_agent("pr402-oracle-health/1")
            .build()
            .expect("reqwest Client::builder().build()")
    })
}

/// True iff [`PR402_SLA_ESCROW_REQUIRE_ORACLE_HEALTHY`] is set to a truthy
/// value (`true`/`1`/`yes`/`on`, case-insensitive).
pub fn gate_enabled() -> bool {
    let raw = p::resolve_string_sync(
        p::PR402_SLA_ESCROW_REQUIRE_ORACLE_HEALTHY,
        p::PR402_SLA_ESCROW_REQUIRE_ORACLE_HEALTHY,
    )
    .unwrap_or_default();
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Derive the oracle's `/health` endpoint from the advertised `registry_url`.
///
/// The convention is that `registry_url` ends in `/v1/registry` (the seller's
/// HMAC-bound upload prefix); `/health` lives at the binary's root. We strip
/// any path that starts with `/v1` and tack on `/health`. When the input is
/// not a parseable URL or the trimmed origin is empty, returns `None`.
pub fn derive_health_url(registry_url: &str) -> Option<String> {
    let trimmed = registry_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    // Find the path-start (first '/' after the scheme://host[:port]).
    let scheme_split = trimmed.find("://")?;
    let after_scheme = &trimmed[scheme_split + 3..];
    let (host_part, path_part) = match after_scheme.find('/') {
        Some(idx) => after_scheme.split_at(idx),
        None => (after_scheme, ""),
    };
    if host_part.is_empty() {
        return None;
    }
    // Strip any /v1... path prefix (covers /v1/registry, /v1/registry/sla, etc.).
    let trimmed_path = if path_part.starts_with("/v1") {
        ""
    } else {
        path_part.trim_end_matches('/')
    };
    let scheme = &trimmed[..scheme_split];
    Some(format!("{scheme}://{host_part}{trimmed_path}/health"))
}

/// Probe an oracle's `/health` and return `Some(error_message)` when the
/// gate is enabled and the probe fails. Returns `None` when:
///
/// - the gate is disabled (default),
/// - `registry_url` is missing or unparseable,
/// - the cached probe is fresh and successful,
/// - a fresh probe succeeds.
///
/// Cached results are reused; new probes update the cache.
pub async fn probe_unhealthy(registry_url: Option<&str>) -> Option<String> {
    if !gate_enabled() {
        return None;
    }
    let registry_url = registry_url?;
    let health_url = derive_health_url(registry_url)?;

    // Cache hit?
    if let Some(hit) = cache().read().await.get(&health_url).cloned() {
        if hit.is_fresh() {
            return hit.result.err();
        }
    }

    // Run the probe.
    let outcome = run_probe(&health_url).await;
    let cached = CachedHealth {
        result: outcome.clone(),
        cached_at: Instant::now(),
    };
    cache().write().await.insert(health_url.clone(), cached);
    match outcome {
        Ok(()) => {
            debug!(target: "oracle_health", %health_url, "probe ok");
            None
        }
        Err(e) => {
            warn!(target: "oracle_health", %health_url, error = %e, "probe failed");
            Some(e)
        }
    }
}

async fn run_probe(health_url: &str) -> Result<(), String> {
    let resp = shared_client()
        .get(health_url)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("non-2xx status {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_health_url_strips_v1_registry_suffix() {
        assert_eq!(
            derive_health_url("https://oracle.example.com/v1/registry"),
            Some("https://oracle.example.com/health".into())
        );
    }

    #[test]
    fn derive_health_url_strips_trailing_slash() {
        assert_eq!(
            derive_health_url("https://oracle.example.com/v1/registry/"),
            Some("https://oracle.example.com/health".into())
        );
    }

    #[test]
    fn derive_health_url_keeps_other_paths() {
        // Operator may run the oracle behind a non-`/v1` path prefix.
        assert_eq!(
            derive_health_url("https://oracle.example.com/api"),
            Some("https://oracle.example.com/api/health".into())
        );
    }

    #[test]
    fn derive_health_url_handles_root_url() {
        assert_eq!(
            derive_health_url("https://oracle.example.com"),
            Some("https://oracle.example.com/health".into())
        );
    }

    #[test]
    fn derive_health_url_rejects_empty() {
        assert_eq!(derive_health_url(""), None);
        assert_eq!(derive_health_url("   "), None);
    }

    #[test]
    fn derive_health_url_rejects_no_scheme() {
        assert_eq!(derive_health_url("oracle.example.com/v1/registry"), None);
    }

    #[test]
    fn derive_health_url_strips_deeper_v1_paths() {
        assert_eq!(
            derive_health_url("https://oracle.example.com/v1/registry/sla"),
            Some("https://oracle.example.com/health".into())
        );
    }

    #[test]
    fn cached_health_is_fresh_within_ttl() {
        let now = CachedHealth {
            result: Ok(()),
            cached_at: Instant::now(),
        };
        assert!(now.is_fresh());
    }
}
