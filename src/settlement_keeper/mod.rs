//! Settlement Keeper — optional post-settlement automation for x402 rails.
//!
//! Normative model: [`oracles/spec/sla-escrow-protocol/v1/NORMATIVE.md`](../../oracles/spec/sla-escrow-protocol/v1/NORMATIVE.md)
//! §6.3 (facilitator MAY run a keeper), §7 (permissionless release/refund), §7.1 (do not assume a keeper).
//!
//! Rail tasks:
//! - **Vault sweeper** — UniversalSettle `Sweep` (exact rail)
//! - **Escrow settler** — SLA-Escrow `ReleasePayment` / `RefundPayment`
//! - **Escrow closer** — SLA-Escrow `ClosePayment` (rent reclamation)

pub mod close;
pub mod config;
pub mod payment;
pub mod sla_escrow;
pub mod sources;
pub mod types;
pub mod vault_sweep;

/// Matches facilitator `LOG_SERVER_LOG` so Vercel `RUST_LOG=pr402=info` shows keeper lines.
pub(crate) const LOG_SERVER_LOG: &str = "server_log";

pub use close::{run_sla_escrow_close, SlaEscrowCloseConfig};
pub use config::KeeperConfig;
pub use sla_escrow::{run_sla_escrow_settle, SlaEscrowSettleConfig};
pub use types::{
    BuildSlaEscrowSettleTxRequest, BuildSlaEscrowSettleTxResponse, SlaEscrowCloseItemResult,
    SlaEscrowCloseOutcome, SlaEscrowCloseRequest, SlaEscrowSettleItemResult,
    SlaEscrowSettleOutcome, SlaEscrowSettleRequest, VaultSweepItemResult, VaultSweepOutcome,
    VaultSweepRequest,
};
pub use vault_sweep::{run_vault_sweep, VaultSweepConfig};
