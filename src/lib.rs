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
pub mod parameters;
pub mod payment_attempt;
pub mod proto;
pub mod scheme;
pub mod sdk;
pub mod seller_provision;
pub mod sla_escrow_payment_build;
pub mod util;
pub mod vault_balance;

pub use facilitator::{Facilitator, FacilitatorLocal};
