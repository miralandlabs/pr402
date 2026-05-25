//! Environment-based configuration for the standalone Settlement Keeper worker.

use std::time::Duration;

use crate::config::Config;
use crate::settlement_keeper::sources::CandidateSourceKind;

#[derive(Debug, Clone)]
pub struct KeeperConfig {
    pub interval: Duration,
    pub sla_escrow_candidate_source: CandidateSourceKind,
    pub vault_sweep_candidate_source: CandidateSourceKind,
    pub database_url: Option<String>,
    pub dry_run: bool,
}

impl KeeperConfig {
    pub fn from_env(base: &Config) -> Result<Self, String> {
        let interval_secs = std::env::var("SETTLEMENT_KEEPER_INTERVAL_SEC")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);

        let sla_source = CandidateSourceKind::parse(
            std::env::var("SETTLEMENT_KEEPER_SLA_ESCROW_SOURCE")
                .ok()
                .as_deref(),
        );
        let sweep_source = CandidateSourceKind::parse(
            std::env::var("SETTLEMENT_KEEPER_VAULT_SWEEP_SOURCE")
                .ok()
                .as_deref(),
        );

        let dry_run = matches!(
            std::env::var("SETTLEMENT_KEEPER_DRY_RUN").ok().as_deref(),
            Some("1" | "true" | "yes")
        );

        let database_url = std::env::var("DATABASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty());

        let _ = base;

        Ok(Self {
            interval: Duration::from_secs(interval_secs),
            sla_escrow_candidate_source: sla_source,
            vault_sweep_candidate_source: sweep_source,
            database_url,
            dry_run,
        })
    }
}
