//! SLAEscrow instruction building for Solana 3.x.
//!
//! This module manually builds SLAEscrow instructions compatible with the
//! steel framework's discriminator format, without depending on Solana 2.x SDKs.

use solana_pubkey::{pubkey, Pubkey};
use solana_transaction::{AccountMeta, Instruction};

/// SPL Associated Token Account program (`spl_associated_token_account::ID`).
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// System Program ID
const SYSTEM_PROGRAM_ID: Pubkey = pubkey!("11111111111111111111111111111111");

/// Token Program ID
const TOKEN_PROGRAM_ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// SLAEscrow instruction discriminators (matches escrow/api/src/instruction.rs).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SLAEscrowInstruction {
    FundPayment = 0,
    ReleasePayment = 1,
    RefundPayment = 2,
    ClosePayment = 3,
    ExtendPaymentTTL = 6,
    ConfirmOracle = 9,
}

/// SLAEscrow FundPayment instruction data structure (176 bytes total).
#[repr(C)]
#[derive(Clone, Debug)]
pub struct FundPaymentData {
    pub seller: Pubkey,           // 32 bytes
    pub mint: Pubkey,             // 32 bytes
    pub amount: [u8; 8],          // 8 bytes (u64 little-endian)
    pub ttl_seconds: [u8; 8],     // 8 bytes (i64 little-endian)
    pub payment_uid: [u8; 32],    // 32 bytes
    pub sla_hash: [u8; 32],       // 32 bytes
    pub oracle_authority: Pubkey, // 32 bytes
}

impl FundPaymentData {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(176);
        bytes.extend_from_slice(&self.seller.to_bytes());
        bytes.extend_from_slice(&self.mint.to_bytes());
        bytes.extend_from_slice(&self.amount);
        bytes.extend_from_slice(&self.ttl_seconds);
        bytes.extend_from_slice(&self.payment_uid);
        bytes.extend_from_slice(&self.sla_hash);
        bytes.extend_from_slice(&self.oracle_authority.to_bytes());
        bytes
    }
}

/// Helper function to sanitize payment_uid for PDA derivation and data structure
pub fn sanitize_uid(uid: &str) -> [u8; 32] {
    let mut uid_bytes = [0u8; 32];
    let uid_str = uid.replace('-', "");
    let uid_bytes_str = uid_str.as_bytes();
    let len = uid_bytes_str.len().min(32);
    uid_bytes[..len].copy_from_slice(&uid_bytes_str[..len]);
    uid_bytes
}

// ----------------------------------------------------------------------------
// PDA Derivations
// ----------------------------------------------------------------------------

pub fn derive_bank_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"bank"], program_id)
}

pub fn derive_config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config"], program_id)
}

pub fn derive_escrow_pda(program_id: &Pubkey, bank_pda: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"escrow", mint.as_ref(), bank_pda.as_ref()], program_id)
}

pub fn derive_payment_pda(
    program_id: &Pubkey,
    bank_pda: &Pubkey,
    payment_uid: &str,
) -> (Pubkey, u8) {
    let uid_bytes = sanitize_uid(payment_uid);
    Pubkey::find_program_address(&[b"payment", &uid_bytes, bank_pda.as_ref()], program_id)
}

pub fn derive_sol_storage_pda(
    program_id: &Pubkey,
    bank_pda: &Pubkey,
    mint: &Pubkey,
    escrow_pda: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"sol_storage",
            mint.as_ref(),
            bank_pda.as_ref(),
            escrow_pda.as_ref(),
        ],
        program_id,
    )
}

pub fn associated_token_address(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            &wallet.to_bytes(),
            &TOKEN_PROGRAM_ID.to_bytes(),
            &mint.to_bytes(),
        ],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

pub fn sla_escrow_token_account(escrow_pda: &Pubkey, mint: &Pubkey) -> Pubkey {
    associated_token_address(escrow_pda, mint)
}

// ----------------------------------------------------------------------------
// Instruction Builders
// ----------------------------------------------------------------------------

/// Build an SLAEscrow FundPayment instruction
#[allow(clippy::too_many_arguments)]
pub fn build_fund_payment_instruction(
    program_id: Pubkey,
    buyer: Pubkey,
    seller: Pubkey,
    mint: Pubkey,
    amount: u64,
    ttl_seconds: i64,
    payment_uid: &str,
    sla_hash: [u8; 32],
    oracle_authority: Pubkey,
) -> Instruction {
    let (bank_pda, _) = derive_bank_pda(&program_id);
    let (config_pda, _) = derive_config_pda(&program_id);
    let (escrow_pda, _) = derive_escrow_pda(&program_id, &bank_pda, &mint);
    let (payment_pda, _) = derive_payment_pda(&program_id, &bank_pda, payment_uid);

    let is_sol = mint == Pubkey::default();

    let data = FundPaymentData {
        seller,
        mint,
        amount: amount.to_le_bytes(),
        ttl_seconds: ttl_seconds.to_le_bytes(),
        payment_uid: sanitize_uid(payment_uid),
        sla_hash,
        oracle_authority,
    };

    let mut instruction_data = Vec::with_capacity(177);
    instruction_data.push(SLAEscrowInstruction::FundPayment as u8);
    instruction_data.extend_from_slice(&data.to_bytes());

    let mut accounts = vec![
        AccountMeta::new(buyer, true),
        AccountMeta::new_readonly(bank_pda, false),
        AccountMeta::new_readonly(config_pda, false),
        AccountMeta::new(escrow_pda, false),
        AccountMeta::new(payment_pda, false),
        AccountMeta::new_readonly(mint, false),
    ];

    if is_sol {
        let (sol_storage_pda, _) =
            derive_sol_storage_pda(&program_id, &bank_pda, &mint, &escrow_pda);
        accounts.push(AccountMeta::new(sol_storage_pda, false));
        accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
    } else {
        let buyer_tokens = associated_token_address(&buyer, &mint);
        let escrow_tokens = sla_escrow_token_account(&escrow_pda, &mint);
        accounts.push(AccountMeta::new(escrow_tokens, false));
        accounts.push(AccountMeta::new(buyer_tokens, false));
        accounts.push(AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false));
        accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
    }

    Instruction {
        program_id,
        accounts,
        data: instruction_data,
    }
}
