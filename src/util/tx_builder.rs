//! Shared Solana transaction-building helpers for exact and SLA-escrow payment builders.
//!
//! These were previously duplicated between [`crate::exact_payment_build`] and
//! [`crate::sla_escrow_payment_build`].

use solana_pubkey::Pubkey;
use solana_transaction::{AccountMeta, Instruction};

use crate::chain::solana::{ASSOCIATED_TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID};

/// Build a `SetComputeUnitLimit` instruction.
pub fn compute_budget_ix_set_limit(units: u32) -> Instruction {
    let mut data = vec![2u8];
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: solana_compute_budget_interface::ID,
        accounts: vec![],
        data,
    }
}

/// Build a `SetComputeUnitPrice` instruction.
pub fn compute_budget_ix_set_price(microlamports_per_cu: u64) -> Instruction {
    let mut data = vec![3u8];
    data.extend_from_slice(&microlamports_per_cu.to_le_bytes());
    Instruction {
        program_id: solana_compute_budget_interface::ID,
        accounts: vec![],
        data,
    }
}

/// Derive Associated Token Account address for an owner/mint/token-program triple.
pub fn associated_token_address(owner: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

/// Build a `CreateIdempotent` instruction for the Associated Token Program.
pub fn create_associated_token_account_idempotent_ix(
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
pub fn system_transfer_ix(from: &Pubkey, to: &Pubkey, lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes()); // Transfer discriminator
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: SYSTEM_PROGRAM_ID,
        accounts: vec![AccountMeta::new(*from, true), AccountMeta::new(*to, false)],
        data,
    }
}

/// Estimate when a recent blockhash will expire (~60s conservative).
pub fn estimate_blockhash_expiry_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now + 60
}

/// Parse a `serde_json::Value` as `u64` (string or number).
pub fn parse_u64_from_json(v: &serde_json::Value, ctx: &str) -> Result<u64, String> {
    match v {
        serde_json::Value::String(s) => s
            .parse()
            .map_err(|_| format!("{}: invalid u64 string", ctx)),
        serde_json::Value::Number(n) => n.as_u64().ok_or_else(|| format!("{}: not u64", ctx)),
        _ => Err(format!("{}: expected string or number", ctx)),
    }
}
