//! Build unsigned refund transactions for pr402-link (and any future refund flow).
//!
//! A refund is a plain `TransferChecked` from the merchant's ATA to the payer's ATA.
//! The merchant wallet is both the authority on the source ATA and the transaction fee payer.
//! No program instruction (no UniversalSettle, no SLA-Escrow) — just a direct SPL transfer.
//!
//! **Instruction order:**
//! 1. `ComputeBudget::SetComputeUnitLimit` (modest — 30k CU)
//! 2. `ComputeBudget::SetComputeUnitPrice`
//! 3. Optional `CreateIdempotent` for payer's destination ATA (funded by merchantWallet)
//! 4. `TransferChecked` from merchantWallet's ATA → payerWallet's ATA
//! 5. Optional `Memo` instruction

use std::str::FromStr;

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use solana_transaction::{
    versioned::VersionedTransaction, Address, Instruction, Message, Transaction,
};

use crate::chain::solana::{SolanaChainProvider, TOKEN_2022_PROGRAM_ID};
use crate::util::tx_builder::{
    associated_token_address, compute_budget_ix_set_limit, compute_budget_ix_set_price,
    create_associated_token_account_idempotent_ix, estimate_blockhash_expiry_unix,
};

use spl_token::solana_program::program_pack::Pack;

/// Memo program v2 (SPL Memo).
const MEMO_PROGRAM_ID: Pubkey =
    solana_pubkey::pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

/// Request body for `POST /api/v1/facilitator/build-refund-tx`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildRefundTxRequest {
    /// Merchant wallet (source of funds AND transaction fee payer).
    pub merchant_wallet: String,
    /// Original payer wallet (refund destination).
    pub payer_wallet: String,
    /// SPL mint address (must be in `PR402_ALLOWED_PAYMENT_MINTS`).
    pub mint: String,
    /// Amount in smallest units (u64 as string or number).
    pub amount: serde_json::Value,
    /// Optional memo to attach to the refund transaction.
    #[serde(default)]
    pub memo: Option<String>,
}

/// Response for `POST /api/v1/facilitator/build-refund-tx`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildRefundTxResponse {
    /// Base64-encoded unsigned `VersionedTransaction` (bincode serialized).
    pub transaction: String,
    /// Index of the merchant wallet in the signers array (always 0 — merchant is fee payer).
    pub payer_signature_index: usize,
    /// Ordered list of required signers. For refunds: `[merchantWallet]`.
    pub signer_pubkeys: Vec<String>,
    /// The blockhash used in the transaction.
    pub recent_blockhash: String,
    /// Estimated UNIX timestamp when the blockhash expires (~60s from build time).
    pub recent_blockhash_expires_at: u64,
    /// Human-readable notes about the transaction.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum RefundTxBuildError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("RPC error: {0}")]
    Rpc(String),
}

/// Build an unsigned refund transaction.
///
/// The caller (pr402-link) is responsible for:
/// 1. Authenticating the merchant (wallet signature on the API call).
/// 2. Verifying the refund is legitimate (link status, amount, etc.).
/// 3. Returning the unsigned tx to the merchant for signing + broadcasting.
pub async fn build_refund_tx(
    provider: &SolanaChainProvider,
    db: Option<&crate::db::Pr402Db>,
    req: BuildRefundTxRequest,
) -> Result<BuildRefundTxResponse, RefundTxBuildError> {
    // ── Parse and validate inputs ───────────────────────────────────
    let merchant_pk = Pubkey::from_str(&req.merchant_wallet)
        .map_err(|e| RefundTxBuildError::InvalidRequest(format!("merchantWallet: {}", e)))?;

    let payer_pk = Pubkey::from_str(&req.payer_wallet)
        .map_err(|e| RefundTxBuildError::InvalidRequest(format!("payerWallet: {}", e)))?;

    let mint_pk = Pubkey::from_str(&req.mint)
        .map_err(|e| RefundTxBuildError::InvalidRequest(format!("mint: {}", e)))?;

    // Validate mint is in the allowlist
    if let Err(msg) = crate::parameters::ensure_allowed_payment_mint(db, &mint_pk).await {
        return Err(RefundTxBuildError::InvalidRequest(msg));
    }

    let amount: u64 = match &req.amount {
        serde_json::Value::String(s) => s
            .parse()
            .map_err(|_| RefundTxBuildError::InvalidRequest("amount: invalid u64 string".into()))?,
        serde_json::Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| RefundTxBuildError::InvalidRequest("amount: not a valid u64".into()))?,
        _ => {
            return Err(RefundTxBuildError::InvalidRequest(
                "amount: expected string or number".into(),
            ))
        }
    };

    if amount == 0 {
        return Err(RefundTxBuildError::InvalidRequest(
            "amount must be > 0".into(),
        ));
    }

    // ── Resolve mint account to determine token program + decimals ───
    let mint_acc = provider
        .rpc_client()
        .get_account(&mint_pk)
        .await
        .map_err(|e| RefundTxBuildError::Rpc(format!("get mint account: {}", e)))?;

    let token_program = if mint_acc.owner == spl_token::ID {
        spl_token::ID
    } else if mint_acc.owner == TOKEN_2022_PROGRAM_ID {
        TOKEN_2022_PROGRAM_ID
    } else {
        return Err(RefundTxBuildError::InvalidRequest(format!(
            "mint owner {} is not Token or Token-2022",
            mint_acc.owner
        )));
    };

    let mint_state = spl_token::state::Mint::unpack(&mint_acc.data)
        .map_err(|_| RefundTxBuildError::InvalidRequest("mint account: unpack failed".into()))?;
    let decimals = mint_state.decimals;

    // ── Get recent blockhash ────────────────────────────────────────
    let blockhash = provider
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| RefundTxBuildError::Rpc(format!("get_latest_blockhash: {}", e)))?;

    // ── Build instructions ──────────────────────────────────────────
    // Modest CU budget — refund is a simple transfer
    let cu_limit = compute_budget_ix_set_limit(30_000);
    let cu_price = compute_budget_ix_set_price(1_000); // 1000 microlamports/CU

    let mut ixs: Vec<Instruction> = vec![cu_limit, cu_price];

    // Derive ATAs
    let source_ata = associated_token_address(&merchant_pk, &mint_pk, &token_program);
    let dest_ata = associated_token_address(&payer_pk, &mint_pk, &token_program);

    // CreateIdempotent for payer's ATA (funded by merchant — merchant self-pays everything)
    // Always include it — idempotent means no harm if it already exists, and it ensures
    // the refund doesn't fail if the payer closed their ATA.
    ixs.push(create_associated_token_account_idempotent_ix(
        &merchant_pk,
        &payer_pk,
        &mint_pk,
        &token_program,
    ));

    // TransferChecked: merchant ATA → payer ATA
    let transfer_ix = spl_token::instruction::transfer_checked(
        &token_program,
        &source_ata,
        &mint_pk,
        &dest_ata,
        &merchant_pk,
        &[],
        amount,
        decimals,
    )
    .map_err(|e| RefundTxBuildError::InvalidRequest(format!("transfer_checked: {}", e)))?;
    ixs.push(transfer_ix);

    // Optional memo
    if let Some(ref memo_text) = req.memo {
        if !memo_text.is_empty() {
            let memo_ix = Instruction {
                program_id: MEMO_PROGRAM_ID,
                accounts: vec![solana_transaction::AccountMeta::new_readonly(
                    merchant_pk,
                    true,
                )],
                data: memo_text.as_bytes().to_vec(),
            };
            ixs.push(memo_ix);
        }
    }

    // ── Assemble unsigned transaction ───────────────────────────────
    // Fee payer = merchantWallet (merchant self-pays the ~5000 lamport tx fee)
    let merchant_addr = Address::new_from_array(merchant_pk.to_bytes());
    let message = Message::new_with_blockhash(&ixs, Some(&merchant_addr), &blockhash);
    let tx = Transaction::new_unsigned(message);
    let vtx = VersionedTransaction::from(tx);

    // Serialize
    let wire = bincode::serialize(&vtx)
        .map_err(|e| RefundTxBuildError::InvalidRequest(format!("bincode serialize: {}", e)))?;
    let tx_b64 = STANDARD.encode(wire);

    let recent_blockhash_expires_at = estimate_blockhash_expiry_unix();

    let notes = vec![
        "Merchant wallet must sign at signatures[0] (fee payer + transfer authority).".into(),
        "Blockhashes expire ~60s: if broadcast fails with BlockhashNotFound, request a fresh build.".into(),
        "The CreateIdempotent instruction is always included (idempotent — safe if ATA exists).".into(),
    ];

    Ok(BuildRefundTxResponse {
        transaction: tx_b64,
        payer_signature_index: 0,
        signer_pubkeys: vec![merchant_pk.to_string()],
        recent_blockhash: blockhash.to_string(),
        recent_blockhash_expires_at,
        notes,
    })
}
