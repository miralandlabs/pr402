//! Shared request/response types for Settlement Keeper runs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SlaEscrowSettleRequest {
    pub limit: Option<u64>,
    pub cooldown_seconds: Option<u64>,
    pub deadline_seconds: Option<u64>,
    pub lookback_seconds: Option<u64>,
    pub dry_run: Option<bool>,
    /// When set, use chain GPA scan instead of pr402 DB for candidate discovery.
    pub candidate_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlaEscrowSettleItemResult {
    pub correlation_id: String,
    pub payment_uid_hex: String,
    pub action: String,
    pub status: String,
    pub signature: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlaEscrowSettleOutcome {
    pub considered: usize,
    pub succeeded: u64,
    pub skipped: u64,
    pub failed: u64,
    pub budget_exhausted_remaining: u64,
    pub items: Vec<SlaEscrowSettleItemResult>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SlaEscrowCloseRequest {
    pub limit: Option<u64>,
    pub cooldown_seconds: Option<u64>,
    pub deadline_seconds: Option<u64>,
    pub dry_run: Option<bool>,
    pub candidate_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlaEscrowCloseItemResult {
    pub payment_uid_hex: String,
    pub action: String,
    pub status: String,
    pub signature: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlaEscrowCloseOutcome {
    pub considered: usize,
    pub succeeded: u64,
    pub skipped: u64,
    pub failed: u64,
    pub budget_exhausted_remaining: u64,
    pub items: Vec<SlaEscrowCloseItemResult>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VaultSweepRequest {
    pub limit: Option<u64>,
    pub cooldown_seconds: Option<u64>,
    pub require_recent_settle_within_seconds: Option<u64>,
    pub dry_run: Option<bool>,
    pub candidate_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSweepItemResult {
    pub wallet: String,
    pub settlement_mode: String,
    pub spl_mint: Option<String>,
    pub available_raw: u64,
    pub threshold_raw: u64,
    pub status: String,
    pub action: String,
    pub signature: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSweepOutcome {
    pub dry_run: bool,
    pub scanned: usize,
    pub attempted: u64,
    pub succeeded: u64,
    pub skipped_below_threshold: u64,
    pub failed: u64,
    pub items: Vec<VaultSweepItemResult>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BuildSlaEscrowSettleTxRequest {
    /// 64-char lowercase hex of on-chain `Payment.payment_uid`.
    pub payment_uid_hex: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSlaEscrowSettleTxResponse {
    pub action: String,
    pub unsigned_transaction: String,
    pub payment_uid_hex: String,
    pub payment_pda: String,
}
