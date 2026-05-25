//! Standalone Settlement Keeper worker — runs vault sweep + escrow settle loops
//! without the Vercel facilitator HTTP surface.

use std::sync::Arc;

use pr402::chain::ChainProvider;
use pr402::config::Config;
use pr402::db::Pr402Db;
use pr402::settlement_keeper::{
    run_sla_escrow_close, run_sla_escrow_settle, run_vault_sweep, KeeperConfig,
    SlaEscrowCloseConfig, SlaEscrowCloseRequest, SlaEscrowSettleConfig, SlaEscrowSettleRequest,
    VaultSweepConfig, VaultSweepRequest,
};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cfg = Config::from_env()?;
    let keeper_cfg = KeeperConfig::from_env(&cfg)?;

    let chain = Arc::new(ChainProvider::from_config(&cfg).await?);
    let db = if let Some(url) = keeper_cfg.database_url.as_deref() {
        Some(Pr402Db::connect(url)?)
    } else {
        std::env::var("DATABASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(Pr402Db::connect)
            .transpose()?
    };

    info!(
        interval_sec = keeper_cfg.interval.as_secs(),
        dry_run = keeper_cfg.dry_run,
        sla_source = ?keeper_cfg.sla_escrow_candidate_source,
        sweep_source = ?keeper_cfg.vault_sweep_candidate_source,
        db = db.is_some(),
        "settlement-keeper started"
    );

    loop {
        if let Some(us) = chain.solana.universalsettle() {
            if let Some(db_ref) = db.as_ref() {
                pr402::parameters::refresh_parameters_from_db(Some(db_ref)).await;
            }
            let sweep_limit = pr402::parameters::resolve_sweep_cron_batch_limit(
                db.as_ref(),
                pr402::parameters::DEFAULT_SWEEP_CRON_BATCH_LIMIT,
            )
            .await;
            let sweep_cooldown = pr402::parameters::resolve_sweep_cron_cooldown_sec(
                db.as_ref(),
                pr402::parameters::DEFAULT_SWEEP_CRON_COOLDOWN_SEC,
            )
            .await;
            let recent_window = pr402::parameters::resolve_sweep_cron_recent_settle_window_sec(
                db.as_ref(),
                pr402::parameters::DEFAULT_SWEEP_CRON_RECENT_SETTLE_WINDOW_SEC,
            )
            .await;

            match run_vault_sweep(
                VaultSweepRequest {
                    dry_run: Some(keeper_cfg.dry_run),
                    candidate_source: Some(
                        format!("{:?}", keeper_cfg.vault_sweep_candidate_source)
                            .to_ascii_lowercase(),
                    ),
                    ..Default::default()
                },
                VaultSweepConfig {
                    chain: &chain,
                    db: db.as_ref(),
                    limit: sweep_limit,
                    cooldown_sec: sweep_cooldown,
                    recent_settle_window_sec: recent_window,
                    dry_run: keeper_cfg.dry_run,
                    candidate_source: keeper_cfg.vault_sweep_candidate_source,
                },
            )
            .await
            {
                Ok(o) => info!(
                    succeeded = o.succeeded,
                    scanned = o.scanned,
                    "vault sweep tick"
                ),
                Err(e) => warn!(error = %e, "vault sweep tick failed"),
            }
            let _ = us;
        }

        if let Some(se) = chain.solana.sla_escrow() {
            if let Some(db_ref) = db.as_ref() {
                pr402::parameters::refresh_parameters_from_db(Some(db_ref)).await;
            }
            let limit = pr402::parameters::resolve_sla_escrow_settle_cron_batch_limit(
                db.as_ref(),
                pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_BATCH_LIMIT,
            )
            .await;
            let cooldown = pr402::parameters::resolve_sla_escrow_settle_cron_cooldown_sec(
                db.as_ref(),
                pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_COOLDOWN_SEC,
            )
            .await;
            let deadline = pr402::parameters::resolve_sla_escrow_settle_cron_deadline_sec(
                db.as_ref(),
                pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_DEADLINE_SEC,
            )
            .await;
            let lookback = pr402::parameters::resolve_sla_escrow_settle_cron_lookback_sec(
                db.as_ref(),
                pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_LOOKBACK_SEC,
            )
            .await;

            match run_sla_escrow_settle(
                SlaEscrowSettleRequest {
                    dry_run: Some(keeper_cfg.dry_run),
                    candidate_source: Some(match keeper_cfg.sla_escrow_candidate_source {
                        pr402::settlement_keeper::sources::CandidateSourceKind::ChainScan => {
                            "chain_scan".into()
                        }
                        pr402::settlement_keeper::sources::CandidateSourceKind::Pr402Db => {
                            "pr402_db".into()
                        }
                    }),
                    ..Default::default()
                },
                SlaEscrowSettleConfig {
                    chain: &chain,
                    db: db.as_ref(),
                    program_id: se.program_id,
                    limit,
                    cooldown_sec: cooldown,
                    deadline_sec: deadline,
                    lookback_sec: lookback,
                    dry_run: keeper_cfg.dry_run,
                    candidate_source: keeper_cfg.sla_escrow_candidate_source,
                },
            )
            .await
            {
                Ok(o) => info!(
                    succeeded = o.succeeded,
                    considered = o.considered,
                    "sla-escrow settle tick"
                ),
                Err(e) => warn!(error = %e, "sla-escrow settle tick failed"),
            }

            match run_sla_escrow_close(
                SlaEscrowCloseRequest {
                    dry_run: Some(keeper_cfg.dry_run),
                    ..Default::default()
                },
                SlaEscrowCloseConfig {
                    chain: &chain,
                    program_id: se.program_id,
                    limit,
                    deadline_sec: deadline,
                    dry_run: keeper_cfg.dry_run,
                    candidate_source: keeper_cfg.sla_escrow_candidate_source,
                },
            )
            .await
            {
                Ok(o) => info!(
                    succeeded = o.succeeded,
                    considered = o.considered,
                    "sla-escrow close tick"
                ),
                Err(e) => warn!(error = %e, "sla-escrow close tick failed"),
            }
        }

        tokio::time::sleep(keeper_cfg.interval).await;
    }
}
