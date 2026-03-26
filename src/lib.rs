//! pr402: Solana-only x402 facilitator for Vercel Serverless
//!
//! This is a minimal implementation of the x402 protocol facilitator,
//! supporting only Solana chain and designed for Vercel Serverless Functions.

pub mod chain;
pub mod config;
pub mod db;
pub mod facilitator;
pub mod onboard_auth;
pub mod parameters;
pub mod payment_attempt;
pub mod proto;
pub mod scheme;
pub mod util;
pub mod vault_balance;

pub use facilitator::{Facilitator, FacilitatorLocal};
