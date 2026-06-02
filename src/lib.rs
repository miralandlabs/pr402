//! pr402: Solana-only x402 facilitator for Vercel Serverless
//!
//! This is a minimal implementation of the x402 protocol facilitator,
//! supporting only Solana chain and designed for Vercel Serverless Functions.
//!
//! **Agents / CLIs:** enable optional feature **`facilitator-http`** for [`sdk::http`] (HTTPS client
//! mirroring `sdk/facilitator-build-tx.ts`). The `facilitator` binary builds without it.

pub mod chain;
pub mod config;
pub mod db;
pub mod exact_payment_build;
pub mod facilitator;
pub mod onboard_auth;
pub mod payable_resource;
#[cfg(feature = "facilitator-http")]
pub mod resource_probe;
#[cfg(not(feature = "facilitator-http"))]
pub mod resource_probe {
    //! Stub when built without `facilitator-http` (no `reqwest`).
    #[derive(Debug, Clone)]
    pub struct ResourceProbeResult {
        pub ok: bool,
        pub http_status: Option<u16>,
        pub scheme: Option<String>,
        pub error: Option<String>,
    }

    pub async fn probe_resource_url(_method: &str, _url: &str) -> ResourceProbeResult {
        ResourceProbeResult {
            ok: false,
            http_status: None,
            scheme: None,
            error: Some("402 probe requires facilitator-http feature".into()),
        }
    }
}
#[cfg(feature = "facilitator-http")]
pub mod oracle_health;
#[cfg(not(feature = "facilitator-http"))]
pub mod oracle_health {
    //! Stub: Wave A §3.2 health gate is disabled when built without the
    //! `facilitator-http` feature (no `reqwest` dependency). All entry points
    //! return "no probe attempted" so the gate is a no-op.
    pub fn gate_enabled() -> bool {
        false
    }
    pub fn derive_health_url(_registry_url: &str) -> Option<String> {
        None
    }
    pub async fn probe_unhealthy(_registry_url: Option<&str>) -> Option<String> {
        None
    }
}
pub mod parameters;
pub mod payment_attempt;
pub mod proto;
pub mod refund_tx_build;
pub mod scheme;
pub mod sdk;
pub mod seller_provision;
pub mod settlement_keeper;
pub mod sla_escrow_payment_build;
pub mod sla_escrow_ttl;
pub mod util;
pub mod vault_balance;

pub use facilitator::{Facilitator, FacilitatorLocal};
