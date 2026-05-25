//! Candidate discovery for Settlement Keeper runs.

pub mod chain_scan;
pub mod pr402_db;

use async_trait::async_trait;

use crate::db::{Pr402Db, SlaEscrowSettleCandidate, SweepCandidate};

/// How the keeper discovers work items. Chain is source of truth for decisions;
/// sources are indexes for efficient polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSourceKind {
    Pr402Db,
    ChainScan,
}

impl CandidateSourceKind {
    pub fn parse(s: Option<&str>) -> Self {
        match s.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("chain_scan" | "chain" | "chain-scan") => Self::ChainScan,
            _ => Self::Pr402Db,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pr402Db => "pr402_db",
            Self::ChainScan => "chain_scan",
        }
    }
}

#[async_trait]
pub trait SlaEscrowSettleCandidateSource: Send + Sync {
    async fn list_sla_escrow_settle_candidates(
        &self,
        cooldown_sec: u64,
        lookback_sec: u64,
        limit: u64,
    ) -> Result<Vec<SlaEscrowSettleCandidate>, String>;
}

#[async_trait]
pub trait VaultSweepCandidateSource: Send + Sync {
    async fn list_sweep_candidates(
        &self,
        cooldown_sec: u64,
        recent_settle_window_sec: u64,
        limit: u64,
    ) -> Result<Vec<SweepCandidate>, String>;
}

pub struct Pr402DbSettleSource<'a> {
    pub db: &'a Pr402Db,
}

#[async_trait]
impl SlaEscrowSettleCandidateSource for Pr402DbSettleSource<'_> {
    async fn list_sla_escrow_settle_candidates(
        &self,
        cooldown_sec: u64,
        lookback_sec: u64,
        limit: u64,
    ) -> Result<Vec<SlaEscrowSettleCandidate>, String> {
        self.db
            .list_sla_escrow_settle_candidates(cooldown_sec, lookback_sec, limit)
            .await
            .map_err(|e| e.to_string())
    }
}

pub struct Pr402DbSweepSource<'a> {
    pub db: &'a Pr402Db,
}

#[async_trait]
impl VaultSweepCandidateSource for Pr402DbSweepSource<'_> {
    async fn list_sweep_candidates(
        &self,
        cooldown_sec: u64,
        recent_settle_window_sec: u64,
        limit: u64,
    ) -> Result<Vec<SweepCandidate>, String> {
        self.db
            .list_sweep_candidates(cooldown_sec, recent_settle_window_sec, limit)
            .await
            .map_err(|e| e.to_string())
    }
}
