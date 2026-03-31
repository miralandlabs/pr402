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
//! **Scope (intentionally narrow):** classic Token or Token-2022 mints only — not native SOL system
//! transfers.

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

    const NATIVE_SOL_MINT: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");
    if pay_mint == NATIVE_SOL_MINT {
        return Err(ExactPaymentBuildError::Unsupported(
            "native SOL mint (111111…) requires a system-transfer layout; use a SPL USDC/wSOL mint or build locally"
                .into(),
        ));
    }

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

    // UniversalSettle: provision vault + vault ATA **before** embedding `recentBlockhash` in the
    // unsigned payment shell. Otherwise `/settle` runs JIT setup (slow `send_and_confirm`) after
    // `/verify` and the payer-signed tx often hits BlockhashNotFound when the facilitator submits.
    if let Some(us_config) = provider.universalsettle() {
        let fee_dest = us_config.fee_destination.ok_or_else(|| {
            ExactPaymentBuildError::InvalidRequest(
                "UniversalSettle fee destination not configured".into(),
            )
        })?;
        let fee_bps = us_config.fee_bps.unwrap_or(100);
        provider
            .ensure_vault_setup(&pay_to, &fee_dest, fee_bps, Some(pay_mint))
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

    let source_ata = associated_token_address(&payer_pk, &pay_mint, &token_program);
    let dest_ata = spl_destination_ata(&pay_to, &pay_mint, us_prog, &token_program);

    if !req.skip_source_balance_check {
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

    let cu_limit = compute_budget_ix_set_limit(provider.max_compute_unit_limit());
    let cu_price = compute_budget_ix_set_price(provider.max_compute_unit_price());

    let mut ixs: Vec<Instruction> = vec![cu_limit, cu_price];

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

    let fee_payer = provider.fee_payer();
    let fee_addr = Address::new_from_array(fee_payer.to_bytes());
    let message = Message::new_with_blockhash(&ixs, Some(&fee_addr), &blockhash);
    let tx = Transaction::new_unsigned(message);
    let vtx = VersionedTransaction::from(tx);
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
    ];

    Ok(BuildExactPaymentTxResponse {
        x402_version: 2,
        transaction: tx_b64,
        recent_blockhash: blockhash.to_string(),
        fee_payer: fee_payer.to_string(),
        payer: payer_pk.to_string(),
        verify_body_template,
        notes,
    })
}
