//! UniversalSettle vault sweeper.

use std::mem::size_of;
use std::str::FromStr;
use std::time::Duration;

use solana_commitment_config::CommitmentConfig;
use tracing::{info, warn};

use crate::chain::ChainProvider;
use crate::chain::TxBudget;
use crate::db::Pr402Db;
use crate::settlement_keeper::sources::{
    CandidateSourceKind, Pr402DbSweepSource, VaultSweepCandidateSource,
};
use crate::settlement_keeper::types::{VaultSweepItemResult, VaultSweepOutcome, VaultSweepRequest};
use crate::settlement_keeper::LOG_SERVER_LOG;

pub struct VaultSweepConfig<'a> {
    pub chain: &'a ChainProvider,
    pub db: Option<&'a Pr402Db>,
    pub limit: u64,
    pub cooldown_sec: u64,
    pub recent_settle_window_sec: u64,
    pub dry_run: bool,
    pub candidate_source: CandidateSourceKind,
}

pub async fn run_vault_sweep(
    req: VaultSweepRequest,
    cfg: VaultSweepConfig<'_>,
) -> Result<VaultSweepOutcome, String> {
    let us_config = cfg
        .chain
        .solana
        .universalsettle()
        .ok_or("UniversalSettle not configured")?;
    let _fee_destination = us_config
        .fee_destination
        .ok_or("UniversalSettle fee destination not configured")?;

    let (cfg_pda, _) = cfg.chain.solana.get_config_pda(&us_config.program_id);
    let onchain_us: crate::chain::solana_universalsettle::Config = {
        let mut last_err = None;
        let mut loaded = None;
        for attempt in 1..=3 {
            match cfg.chain.solana.rpc_client().get_account(&cfg_pda).await {
                Ok(acc) => {
                    loaded = Some(acc);
                    break;
                }
                Err(e) => {
                    let msg = e.to_string();
                    last_err = Some(msg.clone());
                    warn!(
                        target: LOG_SERVER_LOG,
                        task = "vault_sweep",
                        attempt,
                        config_pda = %cfg_pda,
                        error = %msg,
                        "UniversalSettle config load failed; retrying"
                    );
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_millis(500 * attempt)).await;
                    }
                }
            }
        }
        let acc = loaded.ok_or_else(|| {
            format!(
                "failed to load UniversalSettle config after retries: {}",
                last_err.unwrap_or_else(|| "unknown error".into())
            )
        })?;
        let need = 8 + size_of::<crate::chain::solana_universalsettle::Config>();
        if acc.data.len() < need {
            return Err("UniversalSettle config account data too small".into());
        }
        *bytemuck::from_bytes::<crate::chain::solana_universalsettle::Config>(&acc.data[8..need])
    };

    let limit = req.limit.unwrap_or(cfg.limit).clamp(1, 500);
    let cooldown_sec = req
        .cooldown_seconds
        .unwrap_or(cfg.cooldown_sec)
        .clamp(1, 86_400);
    let recent_window_sec = req
        .require_recent_settle_within_seconds
        .unwrap_or(cfg.recent_settle_window_sec)
        .clamp(60, 7 * 86_400);
    let dry_run = req.dry_run.unwrap_or(cfg.dry_run);
    let candidate_source = CandidateSourceKind::parse(req.candidate_source.as_deref());

    let candidates = match candidate_source {
        CandidateSourceKind::Pr402Db => {
            let db = cfg
                .db
                .ok_or("DATABASE_URL required for pr402_db sweep candidates")?;
            Pr402DbSweepSource { db }
                .list_sweep_candidates(cooldown_sec, recent_window_sec, limit)
                .await?
        }
        CandidateSourceKind::ChainScan => {
            return Err(
                "chain_scan candidate source is not yet supported for vault sweep; use pr402_db or discovery API"
                    .into(),
            );
        }
    };

    info!(
        target: LOG_SERVER_LOG,
        task = "vault_sweep",
        dry_run,
        limit,
        cooldown_sec,
        recent_window_sec,
        candidate_source = candidate_source.as_str(),
        candidates = candidates.len(),
        "settlement keeper vault sweep cron started"
    );

    let mut attempted = 0u64;
    let mut succeeded = 0u64;
    let mut skipped_below_threshold = 0u64;
    let mut failed = 0u64;
    let mut items = Vec::with_capacity(candidates.len());

    for c in candidates.iter() {
        let seller = match solana_pubkey::Pubkey::from_str(&c.wallet_pubkey) {
            Ok(pk) => pk,
            Err(_) => {
                failed += 1;
                items.push(VaultSweepItemResult {
                    wallet: c.wallet_pubkey.clone(),
                    settlement_mode: c.settlement_mode.clone(),
                    spl_mint: c.spl_mint.clone(),
                    available_raw: 0,
                    threshold_raw: 0,
                    status: "failed".into(),
                    action: "none".into(),
                    signature: None,
                    error: Some("invalid wallet_pubkey".into()),
                });
                continue;
            }
        };

        let mint_opt = if c.settlement_mode == "spl" {
            match c.spl_mint.as_deref() {
                Some(m) => match solana_pubkey::Pubkey::from_str(m) {
                    Ok(pk) => Some(pk),
                    Err(_) => {
                        failed += 1;
                        items.push(VaultSweepItemResult {
                            wallet: c.wallet_pubkey.clone(),
                            settlement_mode: c.settlement_mode.clone(),
                            spl_mint: c.spl_mint.clone(),
                            available_raw: 0,
                            threshold_raw: 0,
                            status: "failed".into(),
                            action: "none".into(),
                            signature: None,
                            error: Some("invalid spl_mint".into()),
                        });
                        continue;
                    }
                },
                None => {
                    failed += 1;
                    items.push(VaultSweepItemResult {
                        wallet: c.wallet_pubkey.clone(),
                        settlement_mode: c.settlement_mode.clone(),
                        spl_mint: c.spl_mint.clone(),
                        available_raw: 0,
                        threshold_raw: 0,
                        status: "failed".into(),
                        action: "none".into(),
                        signature: None,
                        error: Some("spl settlement_mode without spl_mint".into()),
                    });
                    continue;
                }
            }
        } else {
            None
        };

        let token_program_opt = if let Some(mint) = mint_opt {
            match cfg.chain.solana.rpc_client().get_account(&mint).await {
                Ok(mint_acc) if mint_acc.owner == crate::chain::solana::TOKEN_2022_PROGRAM_ID => {
                    Some(crate::chain::solana::TOKEN_2022_PROGRAM_ID)
                }
                Ok(_) => Some(crate::chain::solana::TOKEN_PROGRAM_ID),
                Err(_) => Some(crate::chain::solana::TOKEN_PROGRAM_ID),
            }
        } else {
            None
        };

        let snap = match crate::vault_balance::fetch_universalsettle_vault_snapshot(
            cfg.chain.solana.rpc_client(),
            us_config.program_id,
            seller,
            mint_opt,
            token_program_opt,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                failed += 1;
                items.push(VaultSweepItemResult {
                    wallet: c.wallet_pubkey.clone(),
                    settlement_mode: c.settlement_mode.clone(),
                    spl_mint: c.spl_mint.clone(),
                    available_raw: 0,
                    threshold_raw: 0,
                    status: "failed".into(),
                    action: "none".into(),
                    signature: None,
                    error: Some(format!("vault snapshot failed: {}", e)),
                });
                continue;
            }
        };

        let provider_floor = c.sweep_threshold.unwrap_or(0);
        let global_floor = if let Some(mint) = mint_opt {
            crate::parameters::resolve_sweep_min_spl_raw_for_mint(&mint)
        } else {
            crate::parameters::resolve_u64_sync(
                crate::parameters::PR402_SWEEP_MIN_SPENDABLE_LAMPORTS,
                crate::parameters::PR402_SWEEP_MIN_SPENDABLE_LAMPORTS,
                crate::parameters::DEFAULT_SWEEP_MIN_SPENDABLE_LAMPORTS,
            )
        };
        let threshold = std::cmp::max(provider_floor, global_floor);
        let available = if mint_opt.is_some() {
            snap.spl_amount_raw
        } else {
            snap.spendable_lamports
        };

        if available < threshold {
            skipped_below_threshold += 1;
            items.push(VaultSweepItemResult {
                wallet: c.wallet_pubkey.clone(),
                settlement_mode: c.settlement_mode.clone(),
                spl_mint: c.spl_mint.clone(),
                available_raw: available,
                threshold_raw: threshold,
                status: "skipped".into(),
                action: "below_threshold".into(),
                signature: None,
                error: None,
            });
            continue;
        }

        if dry_run {
            items.push(VaultSweepItemResult {
                wallet: c.wallet_pubkey.clone(),
                settlement_mode: c.settlement_mode.clone(),
                spl_mint: c.spl_mint.clone(),
                available_raw: available,
                threshold_raw: threshold,
                status: "eligible".into(),
                action: "dry_run".into(),
                signature: None,
                error: None,
            });
            continue;
        }

        attempted += 1;
        let is_sol = mint_opt.is_none();
        let token_mint = mint_opt.unwrap_or_default();
        let (fee_sol_recv, fee_token_owner) =
            crate::chain::solana_universalsettle::sweep_fee_destinations(
                &us_config.program_id,
                &onchain_us.fee_destination,
                &seller,
                onchain_us.use_fee_shard,
                onchain_us.shard_count,
            );
        let mut sweep_instructions = Vec::new();
        if let Some(token_program) = token_program_opt {
            maybe_add_create_ata(
                &mut sweep_instructions,
                cfg.chain,
                &seller,
                &token_mint,
                &token_program,
            )
            .await;
            if fee_token_owner != seller {
                maybe_add_create_ata(
                    &mut sweep_instructions,
                    cfg.chain,
                    &fee_token_owner,
                    &token_mint,
                    &token_program,
                )
                .await;
            }
        }
        sweep_instructions.push(
            crate::chain::solana_universalsettle::build_sweep_instruction(
                us_config.program_id,
                cfg.chain.solana.pubkey(),
                snap.split_vault_pda,
                seller,
                fee_sol_recv,
                fee_token_owner,
                token_mint,
                0,
                is_sol,
                token_program_opt,
            ),
        );

        let send_result = async {
            let (recent_blockhash, _) = cfg
                .chain
                .solana
                .rpc_client()
                .get_latest_blockhash_with_commitment(CommitmentConfig::confirmed())
                .await
                .map_err(|e| e.to_string())?;
            let budget = if is_sol {
                TxBudget::SweepSol
            } else if sweep_instructions.len() > 1 {
                TxBudget::SweepSplWithAta
            } else {
                TxBudget::SweepSpl
            };
            let cu_limit = crate::util::tx_builder::compute_budget_ix_set_limit(budget.cu_limit());
            let cu_price = crate::util::tx_builder::compute_budget_ix_set_price(budget.cu_price());
            let mut final_ixs = vec![cu_limit, cu_price];
            final_ixs.extend(sweep_instructions);
            let sweep_tx = solana_transaction::versioned::VersionedTransaction::from(
                solana_transaction::Transaction::new_signed_with_payer(
                    &final_ixs,
                    Some(&cfg.chain.solana.pubkey()),
                    &[cfg.chain.solana.keypair()],
                    recent_blockhash,
                ),
            );
            cfg.chain
                .solana
                .send_sweep_transaction(&sweep_tx)
                .await
                .map_err(|e| e.to_string())
        }
        .await;

        match send_result {
            Ok(sig) => {
                succeeded += 1;
                if let Some(db) = cfg.db {
                    if let Err(e) = db
                        .record_sweep_attempt(
                            &c.wallet_pubkey,
                            &c.settlement_mode,
                            c.spl_mint.as_deref(),
                            Some(&sig.to_string()),
                        )
                        .await
                    {
                        warn!(target: LOG_SERVER_LOG, error = %e, "record_sweep_attempt (success) failed");
                    }
                }
                info!(
                    target: LOG_SERVER_LOG,
                    task = "vault_sweep",
                    wallet = %c.wallet_pubkey,
                    settlement_mode = %c.settlement_mode,
                    spl_mint = ?c.spl_mint,
                    signature = %sig,
                    available_raw = available,
                    "settlement keeper vault sweep tx submitted"
                );
                items.push(VaultSweepItemResult {
                    wallet: c.wallet_pubkey.clone(),
                    settlement_mode: c.settlement_mode.clone(),
                    spl_mint: c.spl_mint.clone(),
                    available_raw: available,
                    threshold_raw: threshold,
                    status: "ok".into(),
                    action: "sweep_submitted".into(),
                    signature: Some(sig.to_string()),
                    error: None,
                });
            }
            Err(e) => {
                failed += 1;
                if let Some(db) = cfg.db {
                    if let Err(db_err) = db
                        .record_sweep_attempt(
                            &c.wallet_pubkey,
                            &c.settlement_mode,
                            c.spl_mint.as_deref(),
                            None,
                        )
                        .await
                    {
                        warn!(target: LOG_SERVER_LOG, error = %db_err, "record_sweep_attempt (failure) failed");
                    }
                }
                warn!(
                    target: LOG_SERVER_LOG,
                    task = "vault_sweep",
                    wallet = %c.wallet_pubkey,
                    settlement_mode = %c.settlement_mode,
                    error = %e,
                    "settlement keeper vault sweep tx failed"
                );
                items.push(VaultSweepItemResult {
                    wallet: c.wallet_pubkey.clone(),
                    settlement_mode: c.settlement_mode.clone(),
                    spl_mint: c.spl_mint.clone(),
                    available_raw: available,
                    threshold_raw: threshold,
                    status: "failed".into(),
                    action: "sweep_error".into(),
                    signature: None,
                    error: Some(e),
                });
            }
        }
    }

    info!(
        target: LOG_SERVER_LOG,
        task = "vault_sweep",
        dry_run,
        scanned = candidates.len(),
        attempted,
        succeeded,
        skipped_below_threshold,
        failed,
        "settlement keeper vault sweep cron finished"
    );

    Ok(VaultSweepOutcome {
        dry_run,
        scanned: candidates.len(),
        attempted,
        succeeded,
        skipped_below_threshold,
        failed,
        items,
    })
}

async fn maybe_add_create_ata(
    instructions: &mut Vec<solana_transaction::Instruction>,
    chain: &ChainProvider,
    owner: &solana_pubkey::Pubkey,
    mint: &solana_pubkey::Pubkey,
    token_program: &solana_pubkey::Pubkey,
) {
    let ata = crate::util::tx_builder::associated_token_address(owner, mint, token_program);
    match chain.solana.account_exists(&ata).await {
        Ok(true) => {}
        Ok(false) => {
            instructions.push(
                crate::util::tx_builder::create_associated_token_account_idempotent_ix(
                    &chain.solana.pubkey(),
                    owner,
                    mint,
                    token_program,
                ),
            );
        }
        Err(e) => {
            warn!(
                target: LOG_SERVER_LOG,
                error = %e,
                ata = %ata,
                owner = %owner,
                mint = %mint,
                "ATA existence check failed before sweep; adding idempotent create instruction"
            );
            instructions.push(
                crate::util::tx_builder::create_associated_token_account_idempotent_ix(
                    &chain.solana.pubkey(),
                    owner,
                    mint,
                    token_program,
                ),
            );
        }
    }
}
