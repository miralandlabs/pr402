//! SLA-Escrow escrow settler — `ReleasePayment` / `RefundPayment`.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use solana_transaction::Instruction;
use tracing::{info, warn};

use crate::chain::solana::SolanaChainProvider;
use crate::chain::solana_sla_escrow::{
    build_refund_payment_instruction, build_release_payment_instruction, parse_payment_uid_hex,
};
use crate::chain::{ChainProvider, TxBudget};
use crate::db::Pr402Db;
use crate::settlement_keeper::payment::{
    decide_settlement_action, decode_payment_view, SettlementAction,
};
use crate::settlement_keeper::sources::chain_scan::ChainScanSettleSource;
use crate::settlement_keeper::sources::{
    CandidateSourceKind, Pr402DbSettleSource, SlaEscrowSettleCandidateSource,
};
use crate::settlement_keeper::types::{
    SlaEscrowSettleItemResult, SlaEscrowSettleOutcome, SlaEscrowSettleRequest,
};
use crate::settlement_keeper::LOG_SERVER_LOG;

pub struct SlaEscrowSettleConfig<'a> {
    pub chain: &'a ChainProvider,
    pub db: Option<&'a Pr402Db>,
    pub program_id: solana_pubkey::Pubkey,
    pub limit: u64,
    pub cooldown_sec: u64,
    pub deadline_sec: u64,
    pub lookback_sec: u64,
    pub dry_run: bool,
    pub candidate_source: CandidateSourceKind,
}

pub async fn run_sla_escrow_settle(
    req: SlaEscrowSettleRequest,
    cfg: SlaEscrowSettleConfig<'_>,
) -> Result<SlaEscrowSettleOutcome, String> {
    let limit = req.limit.unwrap_or(cfg.limit);
    let cooldown_sec = req.cooldown_seconds.unwrap_or(cfg.cooldown_sec);
    let deadline_sec = req.deadline_seconds.unwrap_or(cfg.deadline_sec);
    let lookback_sec = req.lookback_seconds.unwrap_or(cfg.lookback_sec);
    let dry_run = req.dry_run.unwrap_or(cfg.dry_run);
    let source_kind = CandidateSourceKind::parse(req.candidate_source.as_deref().or({
        match cfg.candidate_source {
            CandidateSourceKind::ChainScan => Some("chain_scan"),
            CandidateSourceKind::Pr402Db => None,
        }
    }));

    let candidates: Vec<_> = match source_kind {
        CandidateSourceKind::Pr402Db => {
            let db = cfg
                .db
                .ok_or("DATABASE_URL required for pr402_db candidate source")?;
            let src = Pr402DbSettleSource { db };
            src.list_sla_escrow_settle_candidates(cooldown_sec, lookback_sec, limit)
                .await?
        }
        CandidateSourceKind::ChainScan => {
            let rpc_url = cfg.chain.solana.rpc_url();
            let src = ChainScanSettleSource::new(rpc_url, cfg.program_id);
            src.list_sla_escrow_settle_candidates(cooldown_sec, lookback_sec, limit)
                .await?
        }
    };

    info!(
        target: LOG_SERVER_LOG,
        task = "sla_escrow_settle",
        dry_run,
        limit,
        cooldown_sec,
        lookback_sec,
        deadline_sec,
        candidate_source = source_kind.as_str(),
        candidates = candidates.len(),
        "settlement keeper sla-escrow settle cron started"
    );

    let deadline = Instant::now() + Duration::from_secs(deadline_sec);
    let mut items = Vec::with_capacity(candidates.len());
    let mut succeeded = 0u64;
    let mut skipped = 0u64;
    let mut failed = 0u64;
    let mut budget_exhausted = 0u64;

    let mut prepared: Vec<(usize, [u8; 32], solana_pubkey::Pubkey)> =
        Vec::with_capacity(candidates.len());
    for (idx, c) in candidates.iter().enumerate() {
        let uid_bytes = match parse_payment_uid_hex(&c.payment_uid_hex) {
            Ok(b) => b,
            Err(e) => {
                failed += 1;
                items.push(SlaEscrowSettleItemResult {
                    correlation_id: c.correlation_id.clone(),
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "error".into(),
                    status: "failed".into(),
                    signature: None,
                    error: Some(format!("invalid payment_uid_hex: {}", e)),
                });
                continue;
            }
        };
        let bank_pda = match c.bank_pda.parse::<solana_pubkey::Pubkey>() {
            Ok(b) => b,
            Err(e) => {
                failed += 1;
                items.push(SlaEscrowSettleItemResult {
                    correlation_id: c.correlation_id.clone(),
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "error".into(),
                    status: "failed".into(),
                    signature: None,
                    error: Some(format!("invalid bank_pda: {}", e)),
                });
                continue;
            }
        };
        let (payment_pda, _) = crate::chain::solana_sla_escrow::derive_payment_pda_from_bytes(
            &cfg.program_id,
            &bank_pda,
            &uid_bytes,
        );
        prepared.push((idx, uid_bytes, payment_pda));
    }

    let pdas: Vec<solana_pubkey::Pubkey> = prepared.iter().map(|(_, _, pda)| *pda).collect();
    let accounts = cfg
        .chain
        .solana
        .rpc_client()
        .get_multiple_accounts(&pdas)
        .await
        .map_err(|e| format!("getMultipleAccounts failed: {}", e))?;

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64;

    for ((idx, uid_bytes, _), account_opt) in prepared.iter().zip(accounts.iter()) {
        if Instant::now() >= deadline {
            budget_exhausted = (candidates.len() - *idx) as u64;
            info!(
                target: LOG_SERVER_LOG,
                task = "sla_escrow_settle",
                processed = items.len(),
                remaining = budget_exhausted,
                "settlement keeper sla-escrow settle cron deadline reached"
            );
            break;
        }

        let c = &candidates[*idx];

        if let Some(db) = cfg.db {
            if let Err(e) = db.touch_sla_escrow_settle_attempt(&c.correlation_id).await {
                warn!(error = %e, correlation_id = %c.correlation_id, "touch_sla_escrow_settle_attempt failed (non-fatal)");
            }
        }

        let Some(acc) = account_opt else {
            failed += 1;
            items.push(SlaEscrowSettleItemResult {
                correlation_id: c.correlation_id.clone(),
                payment_uid_hex: c.payment_uid_hex.clone(),
                action: "error".into(),
                status: "failed".into(),
                signature: None,
                error: Some("Payment PDA not found on-chain".into()),
            });
            continue;
        };

        let payment_view = match decode_payment_view(&acc.data) {
            Ok(p) => p,
            Err(e) => {
                failed += 1;
                items.push(SlaEscrowSettleItemResult {
                    correlation_id: c.correlation_id.clone(),
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "error".into(),
                    status: "failed".into(),
                    signature: None,
                    error: Some(format!("Payment decode failed: {}", e)),
                });
                continue;
            }
        };

        if payment_view.state != 0 {
            skipped += 1;
            items.push(SlaEscrowSettleItemResult {
                correlation_id: c.correlation_id.clone(),
                payment_uid_hex: c.payment_uid_hex.clone(),
                action: "skip_already_settled".into(),
                status: "skipped".into(),
                signature: None,
                error: None,
            });
            continue;
        }

        let is_expired = now_unix > payment_view.expires_at;
        let action = decide_settlement_action(&payment_view, is_expired);

        match action {
            SettlementAction::Release => {
                process_settlement_ix(ProcessSettlementIx {
                    chain: cfg.chain,
                    db: cfg.db,
                    correlation_id: &c.correlation_id,
                    payment_uid_hex: &c.payment_uid_hex,
                    dry_run,
                    action_label: "release_payment",
                    ix: build_release_payment_instruction(
                        cfg.program_id,
                        cfg.chain.solana.pubkey(),
                        payment_view.seller,
                        payment_view.mint,
                        uid_bytes,
                        oracle_authority_for(&payment_view),
                    ),
                    items: &mut items,
                    succeeded: &mut succeeded,
                    failed: &mut failed,
                })
                .await;
            }
            SettlementAction::Refund => {
                process_settlement_ix(ProcessSettlementIx {
                    chain: cfg.chain,
                    db: cfg.db,
                    correlation_id: &c.correlation_id,
                    payment_uid_hex: &c.payment_uid_hex,
                    dry_run,
                    action_label: "refund_payment",
                    ix: build_refund_payment_instruction(
                        cfg.program_id,
                        cfg.chain.solana.pubkey(),
                        payment_view.buyer,
                        payment_view.mint,
                        uid_bytes,
                        oracle_authority_for(&payment_view),
                    ),
                    items: &mut items,
                    succeeded: &mut succeeded,
                    failed: &mut failed,
                })
                .await;
            }
            SettlementAction::SkipPreOutcome | SettlementAction::SkipBuyerOnly => {
                skipped += 1;
                items.push(SlaEscrowSettleItemResult {
                    correlation_id: c.correlation_id.clone(),
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "skip_pre_outcome".into(),
                    status: "skipped".into(),
                    signature: None,
                    error: None,
                });
            }
        }
    }

    info!(
        target: LOG_SERVER_LOG,
        task = "sla_escrow_settle",
        dry_run,
        considered = candidates.len(),
        succeeded,
        skipped,
        failed,
        budget_exhausted_remaining = budget_exhausted,
        "settlement keeper sla-escrow settle cron finished"
    );

    Ok(SlaEscrowSettleOutcome {
        considered: candidates.len(),
        succeeded,
        skipped,
        failed,
        budget_exhausted_remaining: budget_exhausted,
        items,
    })
}

fn oracle_authority_for(
    payment: &crate::settlement_keeper::payment::PaymentView,
) -> Option<solana_pubkey::Pubkey> {
    if payment.oracle_fee_bps > 0 && payment.resolution_state != 0 {
        Some(payment.oracle_authority)
    } else {
        None
    }
}

struct ProcessSettlementIx<'a> {
    chain: &'a ChainProvider,
    db: Option<&'a Pr402Db>,
    correlation_id: &'a str,
    payment_uid_hex: &'a str,
    dry_run: bool,
    action_label: &'a str,
    ix: Instruction,
    items: &'a mut Vec<SlaEscrowSettleItemResult>,
    succeeded: &'a mut u64,
    failed: &'a mut u64,
}

async fn process_settlement_ix(args: ProcessSettlementIx<'_>) {
    if args.dry_run {
        args.items.push(SlaEscrowSettleItemResult {
            correlation_id: args.correlation_id.to_string(),
            payment_uid_hex: args.payment_uid_hex.to_string(),
            action: args.action_label.to_string(),
            status: "dry_run".into(),
            signature: None,
            error: None,
        });
        return;
    }

    match send_settlement_tx(&args.chain.solana, args.ix).await {
        Ok(sig) => {
            *args.succeeded += 1;
            info!(
                target: LOG_SERVER_LOG,
                task = "sla_escrow_settle",
                correlation_id = %args.correlation_id,
                payment_uid_hex = %args.payment_uid_hex,
                action = %args.action_label,
                signature = %sig,
                "settlement keeper sla-escrow settle tx submitted"
            );
            args.items.push(SlaEscrowSettleItemResult {
                correlation_id: args.correlation_id.to_string(),
                payment_uid_hex: args.payment_uid_hex.to_string(),
                action: args.action_label.to_string(),
                status: "ok".into(),
                signature: Some(sig.clone()),
                error: None,
            });
            if let Some(db) = args.db {
                record_lifecycle(db, args.correlation_id, args.action_label, &sig, None, None)
                    .await;
            }
        }
        Err(e) => {
            *args.failed += 1;
            warn!(
                target: LOG_SERVER_LOG,
                task = "sla_escrow_settle",
                correlation_id = %args.correlation_id,
                payment_uid_hex = %args.payment_uid_hex,
                action = %args.action_label,
                error = %e,
                "settlement keeper sla-escrow settle tx failed"
            );
            args.items.push(SlaEscrowSettleItemResult {
                correlation_id: args.correlation_id.to_string(),
                payment_uid_hex: args.payment_uid_hex.to_string(),
                action: args.action_label.to_string(),
                status: "failed".into(),
                signature: None,
                error: Some(e),
            });
        }
    }
}

pub async fn send_settlement_tx(
    provider: &SolanaChainProvider,
    ix: Instruction,
) -> Result<String, String> {
    let blockhash = provider
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| e.to_string())?;

    let budget = TxBudget::SweepSpl;
    let cu_limit = crate::util::tx_builder::compute_budget_ix_set_limit(budget.cu_limit());
    let cu_price = crate::util::tx_builder::compute_budget_ix_set_price(budget.cu_price());

    let payer = provider.pubkey();
    let tx = solana_transaction::versioned::VersionedTransaction::from(
        solana_transaction::Transaction::new_signed_with_payer(
            &[cu_limit, cu_price, ix],
            Some(&payer),
            &[provider.keypair()],
            blockhash,
        ),
    );

    provider
        .send_sweep_transaction(&tx)
        .await
        .map_err(|e| e.to_string())
        .map(|sig| sig.to_string())
}

async fn record_lifecycle(
    db: &Pr402Db,
    correlation_id: &str,
    step: &str,
    tx_signature: &str,
    delivery_hash: Option<&str>,
    resolution_state: Option<i16>,
) {
    if let Err(e) = db
        .apply_escrow_lifecycle_step(
            correlation_id,
            step,
            tx_signature,
            delivery_hash,
            resolution_state,
        )
        .await
    {
        warn!(
            target: LOG_SERVER_LOG,
            error = %e,
            correlation_id = %correlation_id,
            step,
            "apply_escrow_lifecycle_step failed (non-fatal)"
        );
    }
}

/// Build an unsigned settlement tx for manual buyer/seller submission.
pub async fn build_settle_tx_for_payment(
    chain: &ChainProvider,
    program_id: solana_pubkey::Pubkey,
    payment_uid_hex: &str,
) -> Result<(String, String, solana_pubkey::Pubkey), String> {
    let uid_bytes = parse_payment_uid_hex(payment_uid_hex)?;
    let (bank_pda, _) = crate::chain::solana_sla_escrow::derive_bank_pda(&program_id);
    let (payment_pda, _) = crate::chain::solana_sla_escrow::derive_payment_pda_from_bytes(
        &program_id,
        &bank_pda,
        &uid_bytes,
    );

    let acc = chain
        .solana
        .rpc_client()
        .get_account(&payment_pda)
        .await
        .map_err(|e| format!("Payment PDA not found: {}", e))?;

    let payment_view = decode_payment_view(&acc.data)?;
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64;
    let is_expired = now_unix > payment_view.expires_at;
    let action = decide_settlement_action(&payment_view, is_expired);

    let ix = match action {
        SettlementAction::Release => (
            "release_payment",
            build_release_payment_instruction(
                program_id,
                chain.solana.pubkey(),
                payment_view.seller,
                payment_view.mint,
                &uid_bytes,
                oracle_authority_for(&payment_view),
            ),
        ),
        SettlementAction::Refund => (
            "refund_payment",
            build_refund_payment_instruction(
                program_id,
                chain.solana.pubkey(),
                payment_view.buyer,
                payment_view.mint,
                &uid_bytes,
                oracle_authority_for(&payment_view),
            ),
        ),
        SettlementAction::SkipPreOutcome | SettlementAction::SkipBuyerOnly => {
            return Err("payment is not yet eligible for permissionless settlement".into());
        }
    };

    let blockhash = chain
        .solana
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| e.to_string())?;

    let budget = TxBudget::SweepSpl;
    let cu_limit = crate::util::tx_builder::compute_budget_ix_set_limit(budget.cu_limit());
    let cu_price = crate::util::tx_builder::compute_budget_ix_set_price(budget.cu_price());

    let tx = solana_transaction::Transaction::new_signed_with_payer(
        &[cu_limit, cu_price, ix.1],
        Some(&chain.solana.pubkey()),
        &[chain.solana.keypair()],
        blockhash,
    );

    let serialized = bincode::serialize(&tx).map_err(|e| e.to_string())?;
    Ok((
        ix.0.to_string(),
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, serialized),
        payment_pda,
    ))
}
