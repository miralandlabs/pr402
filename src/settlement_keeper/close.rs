//! SLA-Escrow `ClosePayment` housekeeping — rent reclamation after terminal state.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::chain::solana_sla_escrow::{build_close_payment_instruction, parse_payment_uid_hex};
use crate::chain::ChainProvider;
use crate::settlement_keeper::payment::{decide_close_action, decode_payment_view, CloseAction};
use crate::settlement_keeper::sla_escrow::send_settlement_tx;
use crate::settlement_keeper::sources::chain_scan::ChainScanCloseSource;
use crate::settlement_keeper::sources::CandidateSourceKind;
use crate::settlement_keeper::types::{
    SlaEscrowCloseItemResult, SlaEscrowCloseOutcome, SlaEscrowCloseRequest,
};

pub struct SlaEscrowCloseConfig<'a> {
    pub chain: &'a ChainProvider,
    pub program_id: solana_pubkey::Pubkey,
    pub limit: u64,
    pub deadline_sec: u64,
    pub dry_run: bool,
    pub candidate_source: CandidateSourceKind,
}

pub async fn run_sla_escrow_close(
    req: SlaEscrowCloseRequest,
    cfg: SlaEscrowCloseConfig<'_>,
) -> Result<SlaEscrowCloseOutcome, String> {
    let limit = req.limit.unwrap_or(cfg.limit);
    let deadline_sec = req.deadline_seconds.unwrap_or(cfg.deadline_sec);
    let dry_run = req.dry_run.unwrap_or(cfg.dry_run);
    let source_kind = CandidateSourceKind::parse(req.candidate_source.as_deref());

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs() as i64;

    let candidates = match source_kind {
        CandidateSourceKind::ChainScan | CandidateSourceKind::Pr402Db => {
            let rpc_url = cfg.chain.solana.rpc_url();
            let src = ChainScanCloseSource::new(rpc_url, cfg.program_id);
            src.scan_close_candidates(now_unix, limit).await?
        }
    };

    let deadline = Instant::now() + Duration::from_secs(deadline_sec);
    let mut items = Vec::with_capacity(candidates.len());
    let mut succeeded = 0u64;
    let mut skipped = 0u64;
    let mut failed = 0u64;
    let mut budget_exhausted = 0u64;

    for (idx, c) in candidates.iter().enumerate() {
        if Instant::now() >= deadline {
            budget_exhausted = (candidates.len() - idx) as u64;
            break;
        }

        let uid_bytes = match parse_payment_uid_hex(&c.payment_uid_hex) {
            Ok(b) => b,
            Err(e) => {
                failed += 1;
                items.push(SlaEscrowCloseItemResult {
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "error".into(),
                    status: "failed".into(),
                    signature: None,
                    error: Some(format!("invalid payment_uid_hex: {}", e)),
                });
                continue;
            }
        };

        let bank_pda = c
            .bank_pda
            .parse::<solana_pubkey::Pubkey>()
            .map_err(|e| format!("invalid bank_pda: {}", e))?;
        let (payment_pda, _) = crate::chain::solana_sla_escrow::derive_payment_pda_from_bytes(
            &cfg.program_id,
            &bank_pda,
            &uid_bytes,
        );

        let acc = match cfg
            .chain
            .solana
            .rpc_client()
            .get_account(&payment_pda)
            .await
        {
            Ok(a) => a,
            Err(e) => {
                failed += 1;
                items.push(SlaEscrowCloseItemResult {
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "error".into(),
                    status: "failed".into(),
                    signature: None,
                    error: Some(format!("Payment PDA fetch failed: {}", e)),
                });
                continue;
            }
        };

        let payment_view = match decode_payment_view(&acc.data) {
            Ok(p) => p,
            Err(e) => {
                failed += 1;
                items.push(SlaEscrowCloseItemResult {
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "error".into(),
                    status: "failed".into(),
                    signature: None,
                    error: Some(format!("Payment decode failed: {}", e)),
                });
                continue;
            }
        };

        match decide_close_action(&payment_view, now_unix) {
            CloseAction::SkipNotTerminal => {
                skipped += 1;
                items.push(SlaEscrowCloseItemResult {
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "skip_not_terminal".into(),
                    status: "skipped".into(),
                    signature: None,
                    error: None,
                });
                continue;
            }
            CloseAction::SkipClosureDelay => {
                skipped += 1;
                items.push(SlaEscrowCloseItemResult {
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "skip_closure_delay".into(),
                    status: "skipped".into(),
                    signature: None,
                    error: None,
                });
                continue;
            }
            CloseAction::Close => {}
        }

        if dry_run {
            items.push(SlaEscrowCloseItemResult {
                payment_uid_hex: c.payment_uid_hex.clone(),
                action: "close_payment".into(),
                status: "dry_run".into(),
                signature: None,
                error: None,
            });
            continue;
        }

        let ix = build_close_payment_instruction(
            cfg.program_id,
            cfg.chain.solana.pubkey(),
            payment_view.buyer,
            payment_view.mint,
            &uid_bytes,
        );

        match send_settlement_tx(&cfg.chain.solana, ix).await {
            Ok(sig) => {
                succeeded += 1;
                items.push(SlaEscrowCloseItemResult {
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "close_payment".into(),
                    status: "ok".into(),
                    signature: Some(sig),
                    error: None,
                });
            }
            Err(e) => {
                failed += 1;
                items.push(SlaEscrowCloseItemResult {
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "close_payment".into(),
                    status: "failed".into(),
                    signature: None,
                    error: Some(e),
                });
            }
        }
    }

    Ok(SlaEscrowCloseOutcome {
        considered: candidates.len(),
        succeeded,
        skipped,
        failed,
        budget_exhausted_remaining: budget_exhausted,
        items,
    })
}
