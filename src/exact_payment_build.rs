//! Build unsigned legacy [`VersionedTransaction`] shells for `v2:solana:exact` SPL `TransferChecked`
//! payments (same instruction pattern as the `x402_pr402_pay` helper in spl-token-balance-serverless).
//!
//! The facilitator fee payer is the transaction fee payer; the buyer (`payer`) must sign before
//! submitting the verify body. [`SolanaChainProvider::sign`] adds the fee-payer signature at settle.
//!
//! **Instruction order:** compute budget **limit** → **price** → optional **CreateIdempotent** for the
//! merchant **destination** ATA (omitted when **UniversalSettle** is configured — vault ATA is created
//! during facilitator settle; included for **direct `payTo` wallet** settlement) → **TransferChecked**.
//!
//! **Native SOL:** When asset is `11111111111111111111111111111111`, a `SystemProgram::Transfer` is
//! emitted targeting the vault's `sol_storage` PDA (UniversalSettle) or `payTo` directly.

use std::str::FromStr;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::json;
use solana_pubkey::Pubkey;
use solana_transaction::{
    versioned::VersionedTransaction, AccountMeta, Address, Instruction, Message, Transaction,
};

use crate::chain::solana::{
    SolanaChainProvider, ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID,
};

use spl_token::solana_program::program_pack::Pack;

const NATIVE_SOL_MINT: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");

/// Request body for `POST /api/v1/facilitator/build-exact-payment-tx` (facilitator serverless binary).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildExactPaymentTxRequest {
    /// Payer authority (buyer) pubkey; must sign the transaction before verify.
    pub payer: String,
    /// One element copied from `402 accepts[]` (must match `payer`'s chosen rail).
    pub accepted: serde_json::Value,
    /// Copied from the `402` body `resource` field (embedded in payment proof).
    pub resource: serde_json::Value,
    /// If `false` (default), require payer source ATA to exist and hold enough tokens.
    #[serde(default)]
    pub skip_source_balance_check: bool,
    /// If `true`, the builder will inject wrap instructions if the payment mint is wrapped SOL.
    pub auto_wrap_sol: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildExactPaymentTxResponse {
    pub x402_version: u8,
    /// `bincode` serialized [`VersionedTransaction`] (legacy), base64 — **unsigned** (default
    /// signatures). The payer signs their key(s); the facilitator fee payer is added at settle.
    pub transaction: String,
    /// Recent blockhash (base58) embedded in the message.
    pub recent_blockhash: String,
    pub fee_payer: String,
    pub payer: String,
    /// BUY-4: Index into `VersionedTransaction.signatures[]` where the **payer** (buyer) must place
    /// their ed25519 signature. The fee-payer signature at index 0 is added by the facilitator at settle.
    pub payer_signature_index: usize,
    /// BUY-3: Estimated UNIX epoch (seconds) when the embedded `recentBlockhash` expires.
    /// Agents should request a fresh build if `now() >= recentBlockhashExpiresAt`.
    pub recent_blockhash_expires_at: u64,
    /// POST this object to `/verify` / `/settle` **after** replacing `paymentPayload.payload.transaction`
    /// with base64(`bincode(VersionedTransaction)`) of the **signed** transaction (same unsigned shell).
    pub verify_body_template: serde_json::Value,
    /// Human-readable reminders for wallet / agent integrations.
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExactPaymentBuildError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("network mismatch (expected {expected}, got {got})")]
    NetworkMismatch { expected: String, got: String },
    #[error("unsupported for auto-build: {0}")]
    Unsupported(String),
    #[error("RPC error: {0}")]
    Rpc(String),
}

fn compute_budget_ix_set_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: solana_compute_budget_interface::ID,
        accounts: vec![],
        data,
    }
}

fn compute_budget_ix_set_price(microlamports_per_cu: u64) -> Instruction {
    let mut data = vec![3u8];
    data.extend_from_slice(&microlamports_per_cu.to_le_bytes());
    Instruction {
        program_id: solana_compute_budget_interface::ID,
        accounts: vec![],
        data,
    }
}

fn associated_token_address(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

fn spl_destination_ata(
    pay_to: &Pubkey,
    mint: &Pubkey,
    universalsettle_program: Option<Pubkey>,
    token_program: &Pubkey,
) -> Pubkey {
    match universalsettle_program {
        Some(pid) => {
            let (vault, _) = Pubkey::find_program_address(&[b"vault", pay_to.as_ref()], &pid);
            associated_token_address(&vault, mint, token_program)
        }
        None => associated_token_address(pay_to, mint, token_program),
    }
}

fn create_associated_token_account_idempotent_ix(
    funding: &Pubkey,
    owner: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*funding, true),
            AccountMeta::new(associated_token_address(owner, mint, token_program), false),
            AccountMeta::new_readonly(*owner, false),
            AccountMeta::new_readonly(*mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(*token_program, false),
        ],
        data: vec![1],
    }
}

/// Build a `SystemProgram::Transfer` instruction for native SOL.
fn system_transfer_ix(from: &Pubkey, to: &Pubkey, lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes()); // Transfer discriminator
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: SYSTEM_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*from, true),
            AccountMeta::new(*to, false),
        ],
        data,
    }
}

/// Resolve the SOL destination: vault `sol_storage` PDA when UniversalSettle is active, otherwise `pay_to`.
fn sol_destination(pay_to: &Pubkey, provider: &SolanaChainProvider) -> Pubkey {
    if let Some(us_config) = provider.universalsettle() {
        let (vault, _) =
            Pubkey::find_program_address(&[b"vault", pay_to.as_ref()], &us_config.program_id);
        let (sol_storage, _) =
            Pubkey::find_program_address(&[b"sol_storage", vault.as_ref()], &us_config.program_id);
        sol_storage
    } else {
        *pay_to
    }
}

/// Build an unsigned payment transaction and a verify-body template.
pub async fn build_exact_spl_payment_tx(
    provider: &SolanaChainProvider,
    req: BuildExactPaymentTxRequest,
) -> Result<BuildExactPaymentTxResponse, ExactPaymentBuildError> {
    let payer_pk = Pubkey::from_str(&req.payer)
        .map_err(|e| ExactPaymentBuildError::InvalidRequest(format!("payer: {}", e)))?;

    let scheme = req
        .accepted
        .get("scheme")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if scheme != "exact" {
        return Err(ExactPaymentBuildError::InvalidRequest(format!(
            "scheme must be \"exact\", got {:?}",
            scheme
        )));
    }

    let expected_network = provider.chain_id().to_string();
    let got_net = req
        .accepted
        .get("network")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if got_net != expected_network {
        return Err(ExactPaymentBuildError::NetworkMismatch {
            expected: expected_network,
            got: got_net.to_string(),
        });
    }

    let pay_to_str = req
        .accepted
        .get("payTo")
        .and_then(|x| x.as_str())
        .ok_or_else(|| ExactPaymentBuildError::InvalidRequest("accepted.payTo missing".into()))?;
    let pay_to = Pubkey::from_str(pay_to_str)
        .map_err(|e| ExactPaymentBuildError::InvalidRequest(e.to_string()))?;

    let asset_str = req
        .accepted
        .get("asset")
        .and_then(|x| x.as_str())
        .ok_or_else(|| ExactPaymentBuildError::InvalidRequest("accepted.asset missing".into()))?;
    let pay_mint = Pubkey::from_str(asset_str)
        .map_err(|e| ExactPaymentBuildError::InvalidRequest(e.to_string()))?;

    let amount: u64 = match req.accepted.get("amount") {
        Some(v) if v.as_str().is_some() => v.as_str().unwrap().parse().map_err(|_| {
            ExactPaymentBuildError::InvalidRequest("amount: invalid u64 string".into())
        })?,
        Some(v) if v.as_u64().is_some() => v.as_u64().unwrap(),
        _ => {
            return Err(ExactPaymentBuildError::InvalidRequest(
                "accepted.amount missing or not string/u64".into(),
            ))
        }
    };

    let is_native_sol = pay_mint == NATIVE_SOL_MINT;

    // ── Pre-build vault provisioning (UniversalSettle) ───────────────
    if let Some(us_config) = provider.universalsettle() {
        let fee_dest = us_config.fee_destination.ok_or_else(|| {
            ExactPaymentBuildError::InvalidRequest(
                "UniversalSettle fee destination not configured".into(),
            )
        })?;
        let fee_bps = us_config.fee_bps.unwrap_or(100);
        let mint_for_setup = if is_native_sol { None } else { Some(pay_mint) };
        provider
            .ensure_vault_setup(&pay_to, &fee_dest, fee_bps, mint_for_setup)
            .await
            .map_err(|e| {
                ExactPaymentBuildError::Rpc(format!("ensure_vault_setup (pre-build): {e}"))
            })?;
    }

    let blockhash = provider
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| ExactPaymentBuildError::Rpc(e.to_string()))?;

    // ── Build instructions ──────────────────────────────────────────
    let cu_limit = compute_budget_ix_set_limit(provider.max_compute_unit_limit());
    let cu_price = compute_budget_ix_set_price(provider.max_compute_unit_price());
    let mut ixs: Vec<Instruction> = vec![cu_limit, cu_price];

    if is_native_sol {
        // ── Native SOL path: SystemProgram::Transfer ────────────────
        if !req.skip_source_balance_check {
            let bal = provider
                .rpc_client()
                .get_balance(&payer_pk)
                .await
                .map_err(|e| {
                    ExactPaymentBuildError::Rpc(format!("payer SOL balance check: {}", e))
                })?;
            if bal < amount {
                return Err(ExactPaymentBuildError::InvalidRequest(format!(
                    "payer SOL balance {} lamports < required {}",
                    bal, amount
                )));
            }
        }

        let dest = sol_destination(&pay_to, provider);
        ixs.push(system_transfer_ix(&payer_pk, &dest, amount));
    } else {
        // ── SPL token path: TransferChecked ─────────────────────────
        let us_prog = provider.universalsettle().map(|c| c.program_id);

        let mint_acc = provider
            .rpc_client()
            .get_account(&pay_mint)
            .await
            .map_err(|e| ExactPaymentBuildError::Rpc(e.to_string()))?;

        let token_program = if mint_acc.owner == spl_token::ID {
            spl_token::ID
        } else if mint_acc.owner == TOKEN_2022_PROGRAM_ID {
            TOKEN_2022_PROGRAM_ID
        } else {
            return Err(ExactPaymentBuildError::InvalidRequest(format!(
                "mint owner {} is not Token or Token-2022",
                mint_acc.owner
            )));
        };

        let mint_state = spl_token::state::Mint::unpack(&mint_acc.data).map_err(|_| {
            ExactPaymentBuildError::InvalidRequest("mint account: unpack failed".into())
        })?;
        let decimals = mint_state.decimals;

        let source_ata = associated_token_address(&payer_pk, &pay_mint, &token_program);
        let dest_ata = spl_destination_ata(&pay_to, &pay_mint, us_prog, &token_program);

        let auto_wrap = req.auto_wrap_sol.unwrap_or(false);
        const WSOL_MINT: Pubkey = solana_pubkey::pubkey!("So11111111111111111111111111111111111111112");

        if pay_mint == WSOL_MINT && auto_wrap {
            // BUY-5: Auto-wrap wSOL injected by the builder
            ixs.push(create_associated_token_account_idempotent_ix(
                &payer_pk,
                &payer_pk,
                &WSOL_MINT,
                &spl_token::ID,
            ));
            ixs.push(system_transfer_ix(&payer_pk, &source_ata, amount));
            ixs.push(
                spl_token::instruction::sync_native(&spl_token::ID, &source_ata)
                    .map_err(|e| ExactPaymentBuildError::InvalidRequest(format!("sync_native: {}", e)))?,
            );
        } else if !req.skip_source_balance_check {
            let bal = provider
                .rpc_client()
                .get_token_account_balance(&source_ata)
                .await
                .map_err(|e| {
                    ExactPaymentBuildError::InvalidRequest(format!(
                        "payer source ATA {} (mint {}): {}; fund the payer token account first",
                        source_ata, pay_mint, e
                    ))
                })?;
            let raw: u64 = bal
                .amount
                .parse()
                .map_err(|_| ExactPaymentBuildError::Rpc("could not parse token balance".into()))?;
            if raw < amount {
                return Err(ExactPaymentBuildError::InvalidRequest(format!(
                    "payer balance {} raw < required {} (ATA {})",
                    raw, amount, source_ata
                )));
            }
        }

        let need_create_dest = if us_prog.is_some() {
            false
        } else {
            provider.rpc_client().get_account(&dest_ata).await.is_err()
        };

        if need_create_dest {
            ixs.push(create_associated_token_account_idempotent_ix(
                &payer_pk,
                &pay_to,
                &pay_mint,
                &token_program,
            ));
        }

        let transfer_ix = spl_token::instruction::transfer_checked(
            &token_program,
            &source_ata,
            &pay_mint,
            &dest_ata,
            &payer_pk,
            &[],
            amount,
            decimals,
        )
        .map_err(|e| ExactPaymentBuildError::InvalidRequest(format!("transfer_checked: {}", e)))?;
        ixs.push(transfer_ix);
    }

    // ── Assemble unsigned transaction ───────────────────────────────
    let fee_payer = provider.fee_payer();
    let fee_addr = Address::new_from_array(fee_payer.to_bytes());
    let message = Message::new_with_blockhash(&ixs, Some(&fee_addr), &blockhash);
    let tx = Transaction::new_unsigned(message);
    let vtx = VersionedTransaction::from(tx);

    // BUY-4: Determine payer signature index from account keys.
    let payer_signature_index = vtx
        .message
        .static_account_keys()
        .iter()
        .position(|k| *k == payer_pk)
        .unwrap_or(1); // fee_payer is 0; payer is typically 1

    // BUY-3: Estimate blockhash expiry (~60s conservative, Solana slots are ~400ms).
    let recent_blockhash_expires_at = {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now + 60 // conservative: Solana blockhashes last ~60-90s
    };

    let wire = bincode::serialize(&vtx)
        .map_err(|e| ExactPaymentBuildError::InvalidRequest(format!("bincode serialize: {}", e)))?;
    let tx_b64 = STANDARD.encode(wire);

    let verify_body_template = json!({
        "x402Version": 2,
        "paymentPayload": {
            "x402Version": 2,
            "accepted": req.accepted,
            "payload": { "transaction": tx_b64 },
            "resource": req.resource,
            "extensions": {}
        },
        "paymentRequirements": req.accepted,
    });

    let notes = vec![
        "Transaction is unsigned: sign with the payer keypair, then replace paymentPayload.payload.transaction with base64(bincode(VersionedTransaction)) of the signed tx.".into(),
        "Blockhashes expire: if verify/settle fails with BlockhashNotFound, request a fresh build.".into(),
        format!("Payer must sign at signatures[{}]; fee payer (index 0) is added by the facilitator at settle.", payer_signature_index),
    ];

    Ok(BuildExactPaymentTxResponse {
        x402_version: 2,
        transaction: tx_b64,
        recent_blockhash: blockhash.to_string(),
        fee_payer: fee_payer.to_string(),
        payer: payer_pk.to_string(),
        payer_signature_index,
        recent_blockhash_expires_at,
        verify_body_template,
        notes,
    })
}
