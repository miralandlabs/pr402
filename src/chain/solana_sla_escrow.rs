//! SLAEscrow instruction building for Solana 3.x.
//!
//! This module manually builds SLAEscrow instructions compatible with the
//! steel framework's discriminator format, without depending on Solana 2.x SDKs.

use solana_pubkey::{pubkey, Pubkey};
use solana_transaction::{AccountMeta, Instruction};

/// System Program ID
const SYSTEM_PROGRAM_ID: Pubkey = pubkey!("11111111111111111111111111111111");

/// SLAEscrow instruction discriminators (matches escrow/api/src/instruction.rs).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SLAEscrowInstruction {
    FundPayment = 0,
    ReleasePayment = 1,
    RefundPayment = 2,
    ClosePayment = 3,
    ExtendPaymentTTL = 4,
    SubmitDelivery = 5,
    ConfirmOracle = 6,
}

/// SLAEscrow FundPayment instruction data structure (176 bytes total).
#[repr(C)]
#[derive(Clone, Debug)]
pub struct FundPaymentData {
    pub seller: Pubkey,           // 32 bytes (1..33)
    pub mint: Pubkey,             // 32 bytes (33..65)
    pub oracle_authority: Pubkey, // 32 bytes (65..97)
    pub payment_uid: [u8; 32],    // 32 bytes (97..129)
    pub sla_hash: [u8; 32],       // 32 bytes (129..161)
    pub amount: [u8; 8],          // 8 bytes (161..169)
    pub ttl_seconds: [u8; 8],     // 8 bytes (169..177)
}

impl FundPaymentData {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(176);
        bytes.extend_from_slice(&self.seller.to_bytes());
        bytes.extend_from_slice(&self.mint.to_bytes());
        bytes.extend_from_slice(&self.oracle_authority.to_bytes());
        bytes.extend_from_slice(&self.payment_uid);
        bytes.extend_from_slice(&self.sla_hash);
        bytes.extend_from_slice(&self.amount);
        bytes.extend_from_slice(&self.ttl_seconds);
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

/// Parse a 64-lowercase-hex string into the canonical 32-byte
/// `Payment.payment_uid` representation. Strict: rejects mixed case,
/// `0x` prefixes, dashes, spaces, or anything not in `[0-9a-f]`.
///
/// Use this when the caller wants the on-chain `payment_uid` bytes to
/// be the exact 32 bytes whose hex they own — no implicit string
/// sanitization. Pairs with the buyer-authored `TransferSla` whose
/// `payment_uid` field is the same hex (see oracle profile docs).
pub fn parse_payment_uid_hex(s: &str) -> Result<[u8; 32], String> {
    if s.len() != 64 {
        return Err(format!(
            "payment_uid_hex must be exactly 64 chars, got {}",
            s.len()
        ));
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let pair = &s.as_bytes()[i * 2..i * 2 + 2];
        for &c in pair {
            if !matches!(c, b'0'..=b'9' | b'a'..=b'f') {
                return Err(format!(
                    "payment_uid_hex must be lowercase hex (`[0-9a-f]`), invalid byte at offset {}",
                    i * 2
                ));
            }
        }
        let hi = (pair[0] as char).to_digit(16).unwrap() as u8;
        let lo = (pair[1] as char).to_digit(16).unwrap() as u8;
        *byte = (hi << 4) | lo;
    }
    Ok(out)
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
    derive_payment_pda_from_bytes(program_id, bank_pda, &uid_bytes)
}

/// Derive the `Payment` PDA for a 32-byte `payment_uid` (raw bytes,
/// not a string). Used by callers that own canonical hex-encoded uids
/// and don't want the implicit `sanitize_uid` text encoding step.
pub fn derive_payment_pda_from_bytes(
    program_id: &Pubkey,
    bank_pda: &Pubkey,
    payment_uid: &[u8; 32],
) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"payment", payment_uid, bank_pda.as_ref()], program_id)
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

use crate::util::tx_builder::associated_token_address as associated_token_address_with_program;

// ----------------------------------------------------------------------------
// Instruction Builders
// ----------------------------------------------------------------------------

/// Build an SLAEscrow FundPayment instruction.
///
/// `seller` is the **merchant payout wallet** recorded on-chain (`payment.seller`), not the escrow PDA.
/// `token_program` must be `spl_token::ID` or Token-2022 program ID for the mint.
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
    token_program: Pubkey,
) -> Instruction {
    build_fund_payment_instruction_from_uid_bytes(
        program_id,
        buyer,
        seller,
        mint,
        amount,
        ttl_seconds,
        &sanitize_uid(payment_uid),
        sla_hash,
        oracle_authority,
        token_program,
    )
}

/// Variant of [`build_fund_payment_instruction`] that takes the raw
/// 32-byte `payment_uid` directly. Use this when the caller wants the
/// on-chain `Payment.payment_uid` to be a specific 32-byte value (e.g.
/// the bytes whose hex appears in a buyer-authored
/// `TransferSla.payment_uid` field).
#[allow(clippy::too_many_arguments)]
pub fn build_fund_payment_instruction_from_uid_bytes(
    program_id: Pubkey,
    buyer: Pubkey,
    seller: Pubkey,
    mint: Pubkey,
    amount: u64,
    ttl_seconds: i64,
    payment_uid: &[u8; 32],
    sla_hash: [u8; 32],
    oracle_authority: Pubkey,
    token_program: Pubkey,
) -> Instruction {
    let (bank_pda, _) = derive_bank_pda(&program_id);
    let (config_pda, _) = derive_config_pda(&program_id);
    let (escrow_pda, _) = derive_escrow_pda(&program_id, &bank_pda, &mint);
    let (payment_pda, _) = derive_payment_pda_from_bytes(&program_id, &bank_pda, payment_uid);

    let is_sol = mint == Pubkey::default();

    let data = FundPaymentData {
        seller,
        mint,
        amount: amount.to_le_bytes(),
        ttl_seconds: ttl_seconds.to_le_bytes(),
        payment_uid: *payment_uid,
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
        let buyer_tokens = associated_token_address_with_program(&buyer, &mint, &token_program);
        let escrow_tokens =
            associated_token_address_with_program(&escrow_pda, &mint, &token_program);
        accounts.push(AccountMeta::new(escrow_tokens, false));
        accounts.push(AccountMeta::new(buyer_tokens, false));
        accounts.push(AccountMeta::new_readonly(token_program, false));
        accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
    }

    Instruction {
        program_id,
        accounts,
        data: instruction_data,
    }
}

/// SLAEscrow ConfirmOracle instruction data structure
#[repr(C)]
#[derive(Clone, Debug)]
pub struct ConfirmOracleData {
    pub delivery_hash: [u8; 32],
    pub resolution_hash: [u8; 32],
    pub resolution_reason: [u8; 2],
    pub resolution_state: u8,
    pub _padding: [u8; 5],
}

impl ConfirmOracleData {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(72);
        bytes.extend_from_slice(&self.delivery_hash);
        bytes.extend_from_slice(&self.resolution_hash);
        bytes.extend_from_slice(&self.resolution_reason);
        bytes.push(self.resolution_state);
        bytes.extend_from_slice(&self._padding);
        bytes
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_confirm_oracle_instruction(
    program_id: Pubkey,
    oracle_authority: Pubkey,
    mint: Pubkey,
    payment_uid: &str,
    delivery_hash: [u8; 32],
    resolution_hash: [u8; 32],
    resolution_state: u8,
    resolution_reason: u16,
) -> Instruction {
    let (bank_pda, _) = derive_bank_pda(&program_id);
    let (config_pda, _) = derive_config_pda(&program_id);
    let (escrow_pda, _) = derive_escrow_pda(&program_id, &bank_pda, &mint);
    let (payment_pda, _) = derive_payment_pda(&program_id, &bank_pda, payment_uid);

    let data = ConfirmOracleData {
        delivery_hash,
        resolution_hash,
        resolution_reason: resolution_reason.to_le_bytes(),
        resolution_state,
        _padding: [0; 5],
    };

    let mut instruction_data = Vec::with_capacity(73);
    instruction_data.push(SLAEscrowInstruction::ConfirmOracle as u8);
    instruction_data.extend_from_slice(&data.to_bytes());

    let accounts = vec![
        AccountMeta::new(oracle_authority, true),
        AccountMeta::new_readonly(bank_pda, false),
        AccountMeta::new_readonly(config_pda, false),
        AccountMeta::new_readonly(escrow_pda, false),
        AccountMeta::new(payment_pda, false),
    ];

    Instruction {
        program_id,
        accounts,
        data: instruction_data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_payment_uid_hex_round_trips_known_value() {
        let hex_in = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let bytes = parse_payment_uid_hex(hex_in).expect("valid hex");
        assert_eq!(bytes.len(), 32);
        let mut hex_out = String::with_capacity(64);
        use std::fmt::Write;
        for b in bytes {
            write!(&mut hex_out, "{b:02x}").unwrap();
        }
        assert_eq!(hex_out, hex_in);
    }

    #[test]
    fn parse_payment_uid_hex_rejects_short() {
        assert!(parse_payment_uid_hex("abc").is_err());
    }

    #[test]
    fn parse_payment_uid_hex_rejects_uppercase() {
        let s = "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        assert!(parse_payment_uid_hex(s).is_err());
    }

    #[test]
    fn parse_payment_uid_hex_rejects_non_hex() {
        let s = "g123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(parse_payment_uid_hex(s).is_err());
    }

    #[test]
    fn derive_payment_pda_string_and_bytes_agree_when_canonical() {
        // sanitize_uid("uid_123") == ASCII "uid_123" + 25 zero bytes.
        let program_id = Pubkey::new_from_array([1u8; 32]);
        let bank_pda = Pubkey::new_from_array([2u8; 32]);
        let s_pda = derive_payment_pda(&program_id, &bank_pda, "uid_123").0;
        let b_pda =
            derive_payment_pda_from_bytes(&program_id, &bank_pda, &sanitize_uid("uid_123")).0;
        assert_eq!(s_pda, b_pda);
    }
}
