//! SLA-Escrow settlement cron handler.
//!
//! Drives `ReleasePayment` / `RefundPayment` on the four post-outcome
//! permissionless paths defined by sla-escrow program v0.4.0+ (see
//! `oracles/spec/sla-escrow-onchain-abi/v1/NORMATIVE.md` §5.3 / §5.4):
//!
//! - oracle approved (`resolution_state == 1`) → release
//! - oracle rejected (`resolution_state == 2`) → refund
//! - expired + delivered + not rejected → release (expiry path)
//! - expired + no delivery → refund (expiry path)
//!
//! pr402 explicitly does NOT touch the pre-outcome refund path
//! (cooldown gate, buyer-only) — that's reserved for buyer / seller /
//! admin per ABI §5.4.
//!
//! The handler is Vercel-aware: bounded wall-clock budget, batched
//! `getMultipleAccounts` upfront, and fire-and-forget tx submission with
//! preflight to keep the per-candidate cost low. Failed or stale txs
//! are picked up on the next cron tick via the per-row cooldown.

use super::*;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pr402::chain::solana_sla_escrow::{
    build_refund_payment_instruction, build_release_payment_instruction, parse_payment_uid_hex,
};
use pr402::chain::TxBudget;

pub async fn handle_sla_escrow_settle(
    body: Body,
    authorization_header: Option<&str>,
) -> Response<Body> {
    let body_str = match body {
        Body::Text(s) => s,
        Body::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        Body::Empty => "{}".to_string(),
    };
    let req: SlaEscrowSettleRequest = match serde_json::from_str(&body_str) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &format!("Invalid JSON: {}", e)),
    };
    execute_sla_escrow_settle(req, authorization_header).await
}

/// Vercel cron helper (GET-compatible): runs settlement with configured defaults.
pub async fn handle_sla_escrow_settle_cron(authorization_header: Option<&str>) -> Response<Body> {
    execute_sla_escrow_settle(SlaEscrowSettleRequest::default(), authorization_header).await
}

async fn execute_sla_escrow_settle(
    req: SlaEscrowSettleRequest,
    authorization_header: Option<&str>,
) -> Response<Body> {
    if let Err(res) = authorize_sla_escrow_settle(authorization_header).await {
        return res;
    }

    let cp = match CHAIN_PROVIDER.get() {
        Some(c) => c,
        None => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "chain provider not initialized",
            );
        }
    };

    let escrow_config = match cp.solana.sla_escrow() {
        Some(e) => e,
        None => return error_response(StatusCode::BAD_REQUEST, "SLA-Escrow not configured"),
    };
    let program_id = escrow_config.program_id;

    let Some(db) = pr402_db() else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "DATABASE_URL must be configured for SLA-Escrow settlement cron",
        );
    };

    pr402::parameters::refresh_parameters_from_db(Some(db)).await;

    let configured_limit = pr402::parameters::resolve_sla_escrow_settle_cron_batch_limit(
        Some(db),
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_BATCH_LIMIT,
    )
    .await;
    let configured_cooldown = pr402::parameters::resolve_sla_escrow_settle_cron_cooldown_sec(
        Some(db),
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_COOLDOWN_SEC,
    )
    .await;
    let configured_deadline = pr402::parameters::resolve_sla_escrow_settle_cron_deadline_sec(
        Some(db),
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_DEADLINE_SEC,
    )
    .await;
    let configured_lookback = pr402::parameters::resolve_sla_escrow_settle_cron_lookback_sec(
        Some(db),
        pr402::parameters::DEFAULT_SLA_ESCROW_SETTLE_CRON_LOOKBACK_SEC,
    )
    .await;

    let limit = req.limit.unwrap_or(configured_limit);
    let cooldown_sec = req.cooldown_seconds.unwrap_or(configured_cooldown);
    let deadline_sec = req.deadline_seconds.unwrap_or(configured_deadline);
    let lookback_sec = req.lookback_seconds.unwrap_or(configured_lookback);
    let dry_run = req.dry_run.unwrap_or(false);

    let candidates = match db
        .list_sla_escrow_settle_candidates(cooldown_sec, lookback_sec, limit)
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("settle candidate query failed: {}", e),
            )
        }
    };

    info!(
        target: LOG_SERVER_LOG,
        count = candidates.len(),
        cooldown_sec,
        lookback_sec,
        deadline_sec,
        limit,
        dry_run,
        "sla-escrow settle cron started"
    );

    let deadline = Instant::now() + Duration::from_secs(deadline_sec);
    let mut items: Vec<SlaEscrowSettleItemResult> = Vec::with_capacity(candidates.len());
    let mut succeeded = 0u64;
    let mut skipped = 0u64;
    let mut failed = 0u64;
    let mut budget_exhausted = 0u64;

    // Decode all payment_uid_hex values + derive Payment PDAs in one pass.
    // Then fetch all Payment accounts in a single `getMultipleAccounts` call.
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
                    error: Some(format!("invalid payment_uid_hex in DB: {}", e)),
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
                    error: Some(format!("invalid bank_pda in DB: {}", e)),
                });
                continue;
            }
        };
        let (payment_pda, _) = pr402::chain::solana_sla_escrow::derive_payment_pda_from_bytes(
            &program_id,
            &bank_pda,
            &uid_bytes,
        );
        prepared.push((idx, uid_bytes, payment_pda));
    }

    // Batched fetch of all Payment accounts.
    let pdas: Vec<solana_pubkey::Pubkey> = prepared.iter().map(|(_, _, pda)| *pda).collect();
    let accounts = match cp.solana.rpc_client().get_multiple_accounts(&pdas).await {
        Ok(a) => a,
        Err(e) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("getMultipleAccounts failed: {}", e),
            );
        }
    };

    // Read the system clock once for expiry checks. The Solana cluster
    // clock can drift from wall-clock by a few seconds; using system
    // time matches the on-chain `expires_at` semantics closely enough
    // for the cron's purposes (deadline + cooldown ticks are minutes,
    // not seconds). This also saves an RPC call.
    let now_unix = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("system clock read failed: {}", e),
            );
        }
    };

    for ((idx, uid_bytes, _), account_opt) in prepared.iter().zip(accounts.iter()) {
        if Instant::now() >= deadline {
            budget_exhausted = (candidates.len() - *idx) as u64;
            info!(
                target: LOG_SERVER_LOG,
                processed = items.len(),
                remaining = budget_exhausted,
                "sla-escrow settle cron deadline reached; remaining picked up next tick"
            );
            break;
        }

        let c = &candidates[*idx];

        // Touch updated_at regardless of outcome so the per-row cooldown
        // ticks even on skip results. (Batched at end might be cleaner;
        // doing it inline keeps the code simpler.)
        if let Err(e) = db.touch_sla_escrow_settle_attempt(&c.correlation_id).await {
            warn!(
                target: LOG_SERVER_LOG,
                error = %e,
                correlation_id = %c.correlation_id,
                "touch_sla_escrow_settle_attempt failed (non-fatal)"
            );
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

        // Decode the on-chain Payment to make the dispatch decision.
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

        // Already settled.
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
                let mint_pk = payment_view.mint;
                let oracle_authority =
                    if payment_view.oracle_fee_bps > 0 && payment_view.resolution_state != 0 {
                        Some(payment_view.oracle_authority)
                    } else {
                        None
                    };

                if dry_run {
                    items.push(SlaEscrowSettleItemResult {
                        correlation_id: c.correlation_id.clone(),
                        payment_uid_hex: c.payment_uid_hex.clone(),
                        action: "release_payment".into(),
                        status: "dry_run".into(),
                        signature: None,
                        error: None,
                    });
                    continue;
                }

                let ix = build_release_payment_instruction(
                    program_id,
                    cp.solana.pubkey(),
                    payment_view.seller,
                    mint_pk,
                    uid_bytes,
                    oracle_authority,
                );
                match send_settlement_tx(&cp.solana, ix).await {
                    Ok(sig) => {
                        succeeded += 1;
                        items.push(SlaEscrowSettleItemResult {
                            correlation_id: c.correlation_id.clone(),
                            payment_uid_hex: c.payment_uid_hex.clone(),
                            action: "release_payment".into(),
                            status: "ok".into(),
                            signature: Some(sig.clone()),
                            error: None,
                        });
                        record_lifecycle(
                            db,
                            &c.correlation_id,
                            "release_payment",
                            &sig,
                            None,
                            None,
                        )
                        .await;
                    }
                    Err(e) => {
                        failed += 1;
                        items.push(SlaEscrowSettleItemResult {
                            correlation_id: c.correlation_id.clone(),
                            payment_uid_hex: c.payment_uid_hex.clone(),
                            action: "release_payment".into(),
                            status: "failed".into(),
                            signature: None,
                            error: Some(e),
                        });
                    }
                }
            }
            SettlementAction::Refund => {
                let mint_pk = payment_view.mint;
                let oracle_authority =
                    if payment_view.oracle_fee_bps > 0 && payment_view.resolution_state != 0 {
                        Some(payment_view.oracle_authority)
                    } else {
                        None
                    };

                if dry_run {
                    items.push(SlaEscrowSettleItemResult {
                        correlation_id: c.correlation_id.clone(),
                        payment_uid_hex: c.payment_uid_hex.clone(),
                        action: "refund_payment".into(),
                        status: "dry_run".into(),
                        signature: None,
                        error: None,
                    });
                    continue;
                }

                let ix = build_refund_payment_instruction(
                    program_id,
                    cp.solana.pubkey(),
                    payment_view.buyer,
                    mint_pk,
                    uid_bytes,
                    oracle_authority,
                );
                match send_settlement_tx(&cp.solana, ix).await {
                    Ok(sig) => {
                        succeeded += 1;
                        items.push(SlaEscrowSettleItemResult {
                            correlation_id: c.correlation_id.clone(),
                            payment_uid_hex: c.payment_uid_hex.clone(),
                            action: "refund_payment".into(),
                            status: "ok".into(),
                            signature: Some(sig.clone()),
                            error: None,
                        });
                        record_lifecycle(db, &c.correlation_id, "refund_payment", &sig, None, None)
                            .await;
                    }
                    Err(e) => {
                        failed += 1;
                        items.push(SlaEscrowSettleItemResult {
                            correlation_id: c.correlation_id.clone(),
                            payment_uid_hex: c.payment_uid_hex.clone(),
                            action: "refund_payment".into(),
                            status: "failed".into(),
                            signature: None,
                            error: Some(e),
                        });
                    }
                }
            }
            SettlementAction::SkipPreOutcome => {
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
            SettlementAction::SkipBuyerOnly => {
                skipped += 1;
                items.push(SlaEscrowSettleItemResult {
                    correlation_id: c.correlation_id.clone(),
                    payment_uid_hex: c.payment_uid_hex.clone(),
                    action: "skip_pre_outcome_buyer_only".into(),
                    status: "skipped".into(),
                    signature: None,
                    error: None,
                });
            }
        }
    }

    let body = serde_json::json!({
        "considered": candidates.len(),
        "succeeded": succeeded,
        "skipped": skipped,
        "failed": failed,
        "budgetExhaustedRemaining": budget_exhausted,
        "items": items,
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::Text(serde_json::to_string(&body).unwrap_or_default()))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Decision logic per ABI §5.3 / §5.4 + protocol spec §7
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettlementAction {
    Release,
    Refund,
    /// Pre-outcome (oracle still pending) AND not expired. Cron has no
    /// permission to act per protocol spec §7 — we do NOT touch the
    /// pre-cooldown buyer refund path because it's reserved for
    /// buyer / seller / admin, not third-party keepers.
    SkipPreOutcome,
    /// Approved (`resolution_state == 1`) but the cron isn't allowed to
    /// trigger pre-expiry release? Actually post-approval IS
    /// permissionless per ABI §5.3, so this variant is unreachable on
    /// the approve path. Reserved here only to clearly distinguish
    /// the buyer-agency path on Refund/pre-outcome from any future
    /// edge cases. Not currently emitted; left for clarity.
    #[allow(dead_code)]
    SkipBuyerOnly,
}

/// Decide which permissionless settlement path applies given the
/// on-chain state. Implements the v0.4.0 matrix:
///
/// | resolution_state | expired? | delivered? | action |
/// |---|---|---|---|
/// | 1 (Approved) | any | any | Release |
/// | 2 (Rejected) | any | any | Refund |
/// | 0 (Pending) | yes | yes | Release (expired-delivered branch) |
/// | 0 (Pending) | yes | no | Refund (expired-undelivered branch) |
/// | 0 (Pending) | no | any | SkipPreOutcome — buyer/seller/admin only |
fn decide_settlement_action(payment: &PaymentView, is_expired: bool) -> SettlementAction {
    match payment.resolution_state {
        1 => SettlementAction::Release,
        2 => SettlementAction::Refund,
        _ => {
            // resolution_state == 0 (Pending)
            if is_expired {
                if payment.delivery_timestamp != 0 {
                    SettlementAction::Release
                } else {
                    SettlementAction::Refund
                }
            } else {
                SettlementAction::SkipPreOutcome
            }
        }
    }
}

// ---------------------------------------------------------------------------
// On-chain Payment account decoding (per ABI §4.4)
// ---------------------------------------------------------------------------

/// Minimal projection of the on-chain `Payment` struct — only the fields
/// the cron handler needs for dispatch and instruction building.
///
/// Field offsets per `oracles/spec/sla-escrow-onchain-abi/v1/NORMATIVE.md` §4.4
/// (body offsets, after the 8-byte account header per §1.4).
struct PaymentView {
    buyer: solana_pubkey::Pubkey,            // body offset 64
    seller: solana_pubkey::Pubkey,           // body offset 96
    mint: solana_pubkey::Pubkey,             // body offset 128
    oracle_authority: solana_pubkey::Pubkey, // body offset 160
    expires_at: i64,                         // body offset 312
    delivery_timestamp: i64,                 // body offset 328
    oracle_fee_bps: u16,                     // body offset 372
    state: u8,                               // body offset 374
    resolution_state: u8,                    // body offset 375
}

/// Decode a `Payment` account's raw bytes per ABI §4.4. `account.data`
/// is `[disc(1) || padding(7) || body(376)]` per §1.4 — body starts at
/// byte 8.
fn decode_payment_view(data: &[u8]) -> Result<PaymentView, String> {
    const HEADER_LEN: usize = 8;
    const BODY_LEN: usize = 376;
    if data.len() < HEADER_LEN + BODY_LEN {
        return Err(format!(
            "account data too small: expected >= {} bytes, got {}",
            HEADER_LEN + BODY_LEN,
            data.len()
        ));
    }

    // Discriminator MUST be Payment = 103 per ABI §3.
    if data[0] != 103 {
        return Err(format!(
            "account discriminator mismatch: expected 103 (Payment), got {}",
            data[0]
        ));
    }

    let body = &data[HEADER_LEN..HEADER_LEN + BODY_LEN];

    let mut buyer = [0u8; 32];
    buyer.copy_from_slice(&body[64..96]);
    let mut seller = [0u8; 32];
    seller.copy_from_slice(&body[96..128]);
    let mut mint = [0u8; 32];
    mint.copy_from_slice(&body[128..160]);
    let mut oracle = [0u8; 32];
    oracle.copy_from_slice(&body[160..192]);

    let expires_at = i64::from_le_bytes(
        body[312..320]
            .try_into()
            .map_err(|e: std::array::TryFromSliceError| e.to_string())?,
    );
    let delivery_timestamp = i64::from_le_bytes(
        body[328..336]
            .try_into()
            .map_err(|e: std::array::TryFromSliceError| e.to_string())?,
    );
    let oracle_fee_bps = u16::from_le_bytes(
        body[372..374]
            .try_into()
            .map_err(|e: std::array::TryFromSliceError| e.to_string())?,
    );
    let state = body[374];
    let resolution_state = body[375];

    Ok(PaymentView {
        buyer: solana_pubkey::Pubkey::from(buyer),
        seller: solana_pubkey::Pubkey::from(seller),
        mint: solana_pubkey::Pubkey::from(mint),
        oracle_authority: solana_pubkey::Pubkey::from(oracle),
        expires_at,
        delivery_timestamp,
        oracle_fee_bps,
        state,
        resolution_state,
    })
}

// ---------------------------------------------------------------------------
// Tx submission + lifecycle recording
// ---------------------------------------------------------------------------

async fn send_settlement_tx(
    provider: &pr402::chain::solana::SolanaChainProvider,
    ix: solana_transaction::Instruction,
) -> Result<String, String> {
    let blockhash = provider
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| e.to_string())?;

    // Mirror universalsettle sweep budgeting; settlement instructions are
    // small and cheap. CU envelope copies SweepSpl as a conservative
    // upper bound (Release/Refund both transfer tokens + optionally
    // create ATAs).
    let budget = TxBudget::SweepSpl;
    let cu_limit = pr402::util::tx_builder::compute_budget_ix_set_limit(budget.cu_limit());
    let cu_price = pr402::util::tx_builder::compute_budget_ix_set_price(budget.cu_price());

    let payer = provider.pubkey();
    let tx = solana_transaction::versioned::VersionedTransaction::from(
        solana_transaction::Transaction::new_signed_with_payer(
            &[cu_limit, cu_price, ix],
            Some(&payer),
            &[provider.keypair()],
            blockhash,
        ),
    );

    // Fire-and-forget with preflight: matches universalsettle sweep behavior.
    // Failed txs are caught preflight; silent post-submit failures are
    // re-attempted next cron tick via the cooldown.
    provider
        .send_sweep_transaction(&tx)
        .await
        .map_err(|e| e.to_string())
        .map(|sig| sig.to_string())
}

async fn record_lifecycle(
    db: &pr402::db::Pr402Db,
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
            "apply_escrow_lifecycle_step failed (non-fatal); next cron will detect on-chain state"
        );
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

async fn authorize_sla_escrow_settle(header: Option<&str>) -> Result<(), Response<Body>> {
    let expected = pr402::parameters::resolve_sla_escrow_settle_cron_token(pr402_db())
        .await
        .filter(|s| !s.trim().is_empty());
    let Some(expected) = expected else {
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "PR402_SLA_ESCROW_SETTLE_CRON_TOKEN not configured",
        ));
    };
    let Some(supplied) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "missing or malformed bearer token",
        ));
    };
    if supplied.trim() != expected.trim() {
        return Err(error_response(
            StatusCode::UNAUTHORIZED,
            "invalid bearer token",
        ));
    }
    Ok(())
}
