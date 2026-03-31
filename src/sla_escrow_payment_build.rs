//! Build **unsigned** legacy [`VersionedTransaction`] shells for `v2:solana:sla-escrow` [`FundPayment`]
//! (same instruction layout as [`sla_escrow_api::sdk::EscrowSdk::fund_payment`], PDAs resolved from
//! `SLAEscrowConfig.program_id` — not the crate-wired `sla_escrow_api::ID` alone).
//!
//! **Default (Phase 5):** the **facilitator** pays Solana network fees and is listed first as fee payer
//! (same *shape* as [`crate::exact_payment_build`]): the buyer (`payer`) signs as `FundPayment`
//! authority; slot 0 is reserved for the facilitator at `/settle`.
//!
//! Set **`buyer_pays_transaction_fees: true`** for legacy **buyer fee payer** shells (CLI-shaped
//! txs). The facilitator pubkey must still not appear inside instruction account metas.

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
use crate::chain::solana_sla_escrow::build_fund_payment_instruction;
use crate::scheme::v2_solana_escrow::types::SLAEscrowScheme;
use sla_escrow_api::consts::{MAX_TTL_SECONDS, MIN_TTL_SECONDS};

use spl_token::solana_program::program_pack::Pack;

/// Request body for `POST /api/v1/facilitator/build-sla-escrow-payment-tx`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSlaEscrowPaymentTxRequest {
    /// Buyer pubkey; signs `FundPayment` (second signer when the facilitator is fee payer).
    pub payer: String,
    /// One element from `402 accepts[]` (`scheme: "sla-escrow"`).
    pub accepted: serde_json::Value,
    /// From the `402` body `resource` field.
    pub resource: serde_json::Value,
    /// 32-byte SLA terms hash as 64 hex chars (may be all zeros for testing).
    pub sla_hash: String,
    /// Oracle pubkey; must be listed in `accepted.extra.oracleAuthorities`.
    pub oracle_authority: String,
    /// Unique id for this payment (PDA seed). Generated if omitted.
    #[serde(default)]
    pub payment_uid: Option<String>,
    /// If `false` (default), require payer source ATA to exist and hold enough tokens.
    #[serde(default)]
    pub skip_source_balance_check: bool,
    /// If `true`, build a **buyer fee payer** shell (one signer), matching `sla-escrow` CLI.
    /// Default `false` — **facilitator** pays fees (aligned with `build-exact-payment-tx`).
    #[serde(default)]
    pub buyer_pays_transaction_fees: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildSlaEscrowPaymentTxResponse {
    pub x402_version: u8,
    /// Base64 `bincode` [`VersionedTransaction`], **unsigned** (default: facilitator + buyer signer slots).
    pub transaction: String,
    pub recent_blockhash: String,
    /// Network fee payer pubkey (`facilitator` by default; `payer` when `buyerPaysTransactionFees`).
    pub fee_payer: String,
    pub payer: String,
    pub payment_uid: String,
    /// POST to `/verify` / `/settle` after replacing `paymentPayload.payload.transaction` with the
    /// **signed** tx (same message hash / blockhash).
    pub verify_body_template: serde_json::Value,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SlaEscrowPaymentBuildError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("network mismatch (expected {expected}, got {got})")]
    NetworkMismatch { expected: String, got: String },
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("SLA escrow not configured for this facilitator")]
    NotConfigured,
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

fn parse_sla_hash_hex(s: &str) -> Result<[u8; 32], SlaEscrowPaymentBuildError> {
    if s.len() != 64 {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(
            "slaHash must be 64 hex characters".into(),
        ));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let pair = &s[i * 2..i * 2 + 2];
        out[i] = u8::from_str_radix(pair, 16).map_err(|_| {
            SlaEscrowPaymentBuildError::InvalidRequest(format!("slaHash: invalid hex at {}", i))
        })?;
    }
    Ok(out)
}

fn parse_u64_from_json(
    v: &serde_json::Value,
    ctx: &str,
) -> Result<u64, SlaEscrowPaymentBuildError> {
    match v {
        serde_json::Value::String(s) => s.parse().map_err(|_| {
            SlaEscrowPaymentBuildError::InvalidRequest(format!("{}: invalid u64 string", ctx))
        }),
        serde_json::Value::Number(n) => n
            .as_u64()
            .ok_or_else(|| SlaEscrowPaymentBuildError::InvalidRequest(format!("{}: not u64", ctx))),
        _ => Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "{}: expected string or number",
            ctx
        ))),
    }
}

/// Build an unsigned SLA-Escrow fund-payment transaction and verify-body template.
pub async fn build_sla_escrow_fund_payment_tx(
    provider: &SolanaChainProvider,
    req: BuildSlaEscrowPaymentTxRequest,
) -> Result<BuildSlaEscrowPaymentTxResponse, SlaEscrowPaymentBuildError> {
    let escrow_cfg = provider
        .sla_escrow()
        .ok_or(SlaEscrowPaymentBuildError::NotConfigured)?;
    let program_id = escrow_cfg.program_id;

    let payer_pk = Pubkey::from_str(&req.payer)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(format!("payer: {}", e)))?;

    let scheme = req
        .accepted
        .get("scheme")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if scheme != SLAEscrowScheme.as_ref() {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "scheme must be {:?}, got {:?}",
            SLAEscrowScheme.as_ref(),
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
        return Err(SlaEscrowPaymentBuildError::NetworkMismatch {
            expected: expected_network,
            got: got_net.to_string(),
        });
    }

    let pay_to_str = req
        .accepted
        .get("payTo")
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest("accepted.payTo missing".into())
        })?;
    let seller = Pubkey::from_str(pay_to_str)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(e.to_string()))?;

    let asset_str = req
        .accepted
        .get("asset")
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest("accepted.asset missing".into())
        })?;
    let mint = Pubkey::from_str(asset_str)
        .map_err(|e| SlaEscrowPaymentBuildError::InvalidRequest(e.to_string()))?;

    const NATIVE_SOL_MINT: Pubkey = solana_pubkey::pubkey!("11111111111111111111111111111111");
    if mint == Pubkey::default() || mint == NATIVE_SOL_MINT {
        return Err(SlaEscrowPaymentBuildError::Unsupported(
            "SLA-Escrow native SOL fund layout is not supported by this builder; use SPL USDC or build locally"
                .into(),
        ));
    }

    let amount = parse_u64_from_json(
        req.accepted.get("amount").ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest("accepted.amount missing".into())
        })?,
        "accepted.amount",
    )?;

    let ttl_seconds_i64 = parse_u64_from_json(
        req.accepted.get("maxTimeoutSeconds").ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest("accepted.maxTimeoutSeconds missing".into())
        })?,
        "accepted.maxTimeoutSeconds",
    )? as i64;

    if ttl_seconds_i64 < MIN_TTL_SECONDS {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "maxTimeoutSeconds must be >= {} (facilitator verify enforces TTL)",
            MIN_TTL_SECONDS
        )));
    }
    if ttl_seconds_i64 > MAX_TTL_SECONDS {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "maxTimeoutSeconds must be <= {}",
            MAX_TTL_SECONDS
        )));
    }

    let extra = req.accepted.get("extra").ok_or_else(|| {
        SlaEscrowPaymentBuildError::InvalidRequest("accepted.extra missing".into())
    })?;

    let escrow_prog_str = extra
        .get("escrowProgramId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest(
                "accepted.extra.escrowProgramId missing".into(),
            )
        })?;
    let extra_program_id = Pubkey::from_str(escrow_prog_str).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!("escrowProgramId: {}", e))
    })?;
    if extra_program_id != program_id {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "accepted.extra.escrowProgramId ({}) does not match facilitator ESCROW_PROGRAM_ID ({})",
            extra_program_id, program_id
        )));
    }

    let oracle_pk = Pubkey::from_str(&req.oracle_authority).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!("oracleAuthority: {}", e))
    })?;
    let authorities = extra
        .get("oracleAuthorities")
        .and_then(|x| x.as_array())
        .ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest(
                "accepted.extra.oracleAuthorities missing or not an array".into(),
            )
        })?;
    let mut oracle_ok = false;
    for v in authorities {
        let s = v.as_str().ok_or_else(|| {
            SlaEscrowPaymentBuildError::InvalidRequest(
                "accepted.extra.oracleAuthorities entries must be strings".into(),
            )
        })?;
        let p = Pubkey::from_str(s).map_err(|e| {
            SlaEscrowPaymentBuildError::InvalidRequest(format!("oracleAuthorities: {}", e))
        })?;
        if p == oracle_pk {
            oracle_ok = true;
            break;
        }
    }
    if !oracle_ok {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(
            "oracleAuthority is not listed in accepted.extra.oracleAuthorities".into(),
        ));
    }

    let payment_uid = req
        .payment_uid
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ulid::Ulid::new().to_string());

    let sla_hash = parse_sla_hash_hex(&req.sla_hash)?;

    let mint_acc = provider
        .rpc_client()
        .get_account(&mint)
        .await
        .map_err(|e| SlaEscrowPaymentBuildError::Rpc(e.to_string()))?;

    let token_program = if mint_acc.owner == spl_token::ID {
        spl_token::ID
    } else if mint_acc.owner == TOKEN_2022_PROGRAM_ID {
        return Err(SlaEscrowPaymentBuildError::Unsupported(
            "Token-2022 mints require builder support for program-specific FundPayment accounts; use classic Token mints or sla-escrow CLI"
                .into(),
        ));
    } else {
        return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
            "mint owner {} is not Token or Token-2022",
            mint_acc.owner
        )));
    };

    let _decimals = spl_token::state::Mint::unpack(&mint_acc.data)
        .map_err(|_| {
            SlaEscrowPaymentBuildError::InvalidRequest("mint account: unpack failed".into())
        })?
        .decimals;

    let blockhash = provider
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| SlaEscrowPaymentBuildError::Rpc(e.to_string()))?;

    let source_ata = associated_token_address(&payer_pk, &mint, &token_program);

    if !req.skip_source_balance_check {
        let bal = provider
            .rpc_client()
            .get_token_account_balance(&source_ata)
            .await
            .map_err(|e| {
                SlaEscrowPaymentBuildError::InvalidRequest(format!(
                    "payer source ATA {} (mint {}): {}",
                    source_ata, mint, e
                ))
            })?;
        let raw: u64 = bal
            .amount
            .parse()
            .map_err(|_| SlaEscrowPaymentBuildError::Rpc("could not parse token balance".into()))?;
        if raw < amount {
            return Err(SlaEscrowPaymentBuildError::InvalidRequest(format!(
                "payer balance {} raw < required {} (ATA {})",
                raw, amount, source_ata
            )));
        }
    }

    let bank_pda = escrow_cfg.bank_address.ok_or_else(|| {
        SlaEscrowPaymentBuildError::InvalidRequest(
            "facilitator: bank_address not loaded (SLA escrow bank account missing in config)"
                .into(),
        )
    })?;
    let (escrow_pda, _) = provider.get_escrow_pda(mint, bank_pda);
    let escrow_token_ata = associated_token_address(&escrow_pda, &mint, &token_program);

    let cu_limit = compute_budget_ix_set_limit(provider.max_compute_unit_limit());
    let cu_price = compute_budget_ix_set_price(provider.max_compute_unit_price());

    let mut ixs: Vec<Instruction> = vec![cu_limit, cu_price];

    let need_create_escrow_ata = provider
        .rpc_client()
        .get_account(&escrow_token_ata)
        .await
        .is_err();
    if need_create_escrow_ata {
        ixs.push(create_associated_token_account_idempotent_ix(
            &payer_pk,
            &escrow_pda,
            &mint,
            &token_program,
        ));
    }

    let fund_ix = build_fund_payment_instruction(
        program_id,
        payer_pk,
        seller,
        mint,
        amount,
        ttl_seconds_i64,
        &payment_uid,
        sla_hash,
        oracle_pk,
    );
    ixs.push(fund_ix);

    let fee_payer_pk = if req.buyer_pays_transaction_fees {
        payer_pk
    } else {
        provider.fee_payer()
    };
    let fee_payer_addr = Address::new_from_array(fee_payer_pk.to_bytes());
    let message = Message::new_with_blockhash(&ixs, Some(&fee_payer_addr), &blockhash);
    let tx = Transaction::new_unsigned(message);
    let vtx = VersionedTransaction::from(tx);
    let wire = bincode::serialize(&vtx).map_err(|e| {
        SlaEscrowPaymentBuildError::InvalidRequest(format!("bincode serialize: {}", e))
    })?;
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

    let notes = if req.buyer_pays_transaction_fees {
        vec![
            "SLA-Escrow (legacy): buyer pays Solana fees and is the sole signer — facilitator does not appear in instruction accounts.".into(),
            "Sign with the buyer keypair; you may broadcast before verify or use /settle as today.".into(),
            "Blockhashes expire; rebuild if verification fails with BlockhashNotFound.".into(),
        ]
    } else {
        vec![
            "SLA-Escrow (default): facilitator pays Solana fees; buyer signs FundPayment authority (second signer, same pattern as build-exact-payment-tx).".into(),
            "Sign with the buyer keypair only (partial sign); POST /verify then /settle so the facilitator fills fee-payer signature slot 0.".into(),
            "Blockhashes expire; rebuild if verification fails with BlockhashNotFound.".into(),
        ]
    };

    Ok(BuildSlaEscrowPaymentTxResponse {
        x402_version: 2,
        transaction: tx_b64,
        recent_blockhash: blockhash.to_string(),
        fee_payer: fee_payer_pk.to_string(),
        payer: payer_pk.to_string(),
        payment_uid,
        verify_body_template,
        notes,
    })
}
