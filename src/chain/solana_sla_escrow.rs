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

// ----------------------------------------------------------------------------
// Settlement instruction builders (post-v0.4.0 permissionless paths)
// ----------------------------------------------------------------------------
//
// These build `ReleasePayment` (discriminator 1) and `RefundPayment`
// (discriminator 2) per `oracles/spec/sla-escrow-onchain-abi/v1/NORMATIVE.md`
// §5.3 and §5.4. The instructions carry no body bytes — only the 1-byte
// discriminator. PDA seeds and account orderings follow §2.1 and §5.3 / §5.4.
//
// Hand-rolled (rather than using the `sla-escrow-api` SDK) for the same
// multi-cluster reason as `oracle-common::settler.rs`: the SDK pins
// `declare_id!` at compile time, but pr402 needs to support both mainnet
// and devnet from one Vercel deployment by passing `program_id` at runtime.

/// Token program ID for SPL Token (classic). Token-2022 not yet
/// supported on the settlement path; integrators MUST pass classic SPL
/// or call out to a different builder when adding Token-2022 support.
const SPL_TOKEN_PROGRAM_ID: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

/// Associated token account program.
const SPL_ATA_PROGRAM_ID: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/// Build an SLAEscrow `ReleasePayment` instruction. Sends `payment.amount`
/// (less protocol fee and oracle tip) to `payment.seller`.
///
/// Permissionless once `payment.resolution_state == 1` (oracle approved)
/// or once `now > expires_at` AND delivery was submitted AND not rejected.
/// pr402 runs as the fee-payer signer — no privileged key required.
///
/// `seller` MUST equal the on-chain `payment.seller` (read from the
/// Payment PDA before calling). `mint == Pubkey::default()` selects the
/// SOL path; otherwise SPL.
///
/// `oracle_authority` is REQUIRED in this builder iff `payment.oracle_fee_bps > 0`
/// AND `payment.resolution_state != 0` (a tip will be paid). Pass `None`
/// to omit the optional oracle-tip accounts; pass `Some(authority)` to
/// include them.
#[allow(clippy::too_many_arguments)]
pub fn build_release_payment_instruction(
    program_id: Pubkey,
    caller: Pubkey,
    seller: Pubkey,
    mint: Pubkey,
    payment_uid: &[u8; 32],
    oracle_authority: Option<Pubkey>,
) -> Instruction {
    let (bank_pda, _) = derive_bank_pda(&program_id);
    let (config_pda, _) = derive_config_pda(&program_id);
    let (escrow_pda, _) = derive_escrow_pda(&program_id, &bank_pda, &mint);
    let (payment_pda, _) = derive_payment_pda_from_bytes(&program_id, &bank_pda, payment_uid);

    let is_sol = mint == Pubkey::default();

    let data = vec![SLAEscrowInstruction::ReleasePayment as u8];

    let mut accounts = vec![
        AccountMeta::new(caller, true), // 0: caller (signer, writable)
        AccountMeta::new_readonly(bank_pda, false),
        AccountMeta::new_readonly(config_pda, false),
        AccountMeta::new(escrow_pda, false),
        AccountMeta::new(payment_pda, false),
        AccountMeta::new_readonly(mint, false),
    ];

    if is_sol {
        // SOL path per ABI §5.3:
        // [caller, bank, config, escrow, payment, mint, sol_storage, seller, system_program]
        // optional [oracle_authority] when oracle tip due
        let (sol_storage_pda, _) =
            derive_sol_storage_pda(&program_id, &bank_pda, &mint, &escrow_pda);
        accounts.push(AccountMeta::new(sol_storage_pda, false));
        accounts.push(AccountMeta::new(seller, false));
        accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
        if let Some(oracle) = oracle_authority {
            accounts.push(AccountMeta::new(oracle, false));
        }
    } else {
        // SPL path per ABI §5.3:
        // [caller, bank, config, escrow, payment, mint, escrow_tokens, seller_tokens, seller,
        //  token_program, ata_program, system_program]
        // optional [oracle_tokens, oracle_authority] when oracle tip due
        let escrow_tokens =
            associated_token_address_with_program(&escrow_pda, &mint, &SPL_TOKEN_PROGRAM_ID);
        let seller_tokens =
            associated_token_address_with_program(&seller, &mint, &SPL_TOKEN_PROGRAM_ID);
        accounts.push(AccountMeta::new(escrow_tokens, false));
        accounts.push(AccountMeta::new(seller_tokens, false));
        accounts.push(AccountMeta::new(seller, false));
        accounts.push(AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false));
        accounts.push(AccountMeta::new_readonly(SPL_ATA_PROGRAM_ID, false));
        accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
        if let Some(oracle) = oracle_authority {
            let oracle_tokens =
                associated_token_address_with_program(&oracle, &mint, &SPL_TOKEN_PROGRAM_ID);
            accounts.push(AccountMeta::new(oracle_tokens, false));
            accounts.push(AccountMeta::new(oracle, false));
        }
    }

    Instruction {
        program_id,
        accounts,
        data,
    }
}

/// Build an SLAEscrow `RefundPayment` instruction. Returns `payment.amount`
/// (less oracle tip if applicable) to `payment.buyer`.
///
/// Permissionless once `payment.resolution_state == 2` (oracle rejected),
/// or once `now > expires_at` with no delivery submitted, or once
/// `now > expires_at` AND `resolution_state == 2`. pr402 MUST NOT call
/// this on the pre-outcome buyer-cooldown path (that's restricted to
/// buyer/seller/admin per ABI §5.4); the cron handler enforces this at
/// the dispatch decision.
///
/// `buyer` MUST equal the on-chain `payment.buyer`. SOL vs SPL selected
/// by `mint == Pubkey::default()`. `oracle_authority` semantics match
/// `build_release_payment_instruction` above.
#[allow(clippy::too_many_arguments)]
pub fn build_refund_payment_instruction(
    program_id: Pubkey,
    caller: Pubkey,
    buyer: Pubkey,
    mint: Pubkey,
    payment_uid: &[u8; 32],
    oracle_authority: Option<Pubkey>,
) -> Instruction {
    let (bank_pda, _) = derive_bank_pda(&program_id);
    let (config_pda, _) = derive_config_pda(&program_id);
    let (escrow_pda, _) = derive_escrow_pda(&program_id, &bank_pda, &mint);
    let (payment_pda, _) = derive_payment_pda_from_bytes(&program_id, &bank_pda, payment_uid);

    let is_sol = mint == Pubkey::default();

    let data = vec![SLAEscrowInstruction::RefundPayment as u8];

    let mut accounts = vec![
        AccountMeta::new(caller, true), // 0: caller (signer, writable)
        AccountMeta::new_readonly(bank_pda, false),
        AccountMeta::new_readonly(config_pda, false),
        AccountMeta::new(escrow_pda, false),
        AccountMeta::new(payment_pda, false),
        AccountMeta::new_readonly(mint, false),
    ];

    if is_sol {
        // SOL path per ABI §5.4:
        // [caller, bank, config, escrow, payment, mint, sol_storage, buyer, system_program]
        // optional [oracle_authority] when oracle tip due
        let (sol_storage_pda, _) =
            derive_sol_storage_pda(&program_id, &bank_pda, &mint, &escrow_pda);
        accounts.push(AccountMeta::new(sol_storage_pda, false));
        accounts.push(AccountMeta::new(buyer, false));
        accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
        if let Some(oracle) = oracle_authority {
            accounts.push(AccountMeta::new(oracle, false));
        }
    } else {
        // SPL path per ABI §5.4:
        // [caller, bank, config, escrow, payment, mint, escrow_tokens, buyer_tokens, token_program]
        // optional [oracle_tokens, oracle_authority, ata_program, system_program] when oracle tip due
        let escrow_tokens =
            associated_token_address_with_program(&escrow_pda, &mint, &SPL_TOKEN_PROGRAM_ID);
        let buyer_tokens =
            associated_token_address_with_program(&buyer, &mint, &SPL_TOKEN_PROGRAM_ID);
        accounts.push(AccountMeta::new(escrow_tokens, false));
        accounts.push(AccountMeta::new(buyer_tokens, false));
        accounts.push(AccountMeta::new_readonly(SPL_TOKEN_PROGRAM_ID, false));
        if let Some(oracle) = oracle_authority {
            let oracle_tokens =
                associated_token_address_with_program(&oracle, &mint, &SPL_TOKEN_PROGRAM_ID);
            accounts.push(AccountMeta::new(oracle_tokens, false));
            accounts.push(AccountMeta::new(oracle, false));
            accounts.push(AccountMeta::new_readonly(SPL_ATA_PROGRAM_ID, false));
            accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
        }
    }

    Instruction {
        program_id,
        accounts,
        data,
    }
}

/// Build an SLAEscrow `ClosePayment` instruction. Permissionless after
/// `payment.closed_at` when the payment is in a terminal state.
pub fn build_close_payment_instruction(
    program_id: Pubkey,
    caller: Pubkey,
    buyer: Pubkey,
    mint: Pubkey,
    payment_uid: &[u8; 32],
) -> Instruction {
    let (bank_pda, _) = derive_bank_pda(&program_id);
    let (config_pda, _) = derive_config_pda(&program_id);
    let (escrow_pda, _) = derive_escrow_pda(&program_id, &bank_pda, &mint);
    let (payment_pda, _) = derive_payment_pda_from_bytes(&program_id, &bank_pda, payment_uid);

    let data = vec![SLAEscrowInstruction::ClosePayment as u8];
    let accounts = vec![
        AccountMeta::new(caller, true),
        AccountMeta::new(buyer, false),
        AccountMeta::new_readonly(bank_pda, false),
        AccountMeta::new_readonly(config_pda, false),
        AccountMeta::new(escrow_pda, false),
        AccountMeta::new(payment_pda, false),
        AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
    ];

    Instruction {
        program_id,
        accounts,
        data,
    }
}

#[cfg(test)]
mod tests_settlement {
    use super::*;

    #[test]
    fn release_sol_no_oracle_tip_account_count() {
        let pid = Pubkey::new_unique();
        let caller = Pubkey::new_unique();
        let seller = Pubkey::new_unique();
        let mint = Pubkey::default();
        let uid = [1u8; 32];
        let ix = build_release_payment_instruction(pid, caller, seller, mint, &uid, None);
        // SOL path: 9 accounts + 0 optional = 9
        assert_eq!(ix.accounts.len(), 9);
        assert_eq!(ix.data, vec![SLAEscrowInstruction::ReleasePayment as u8]);
    }

    #[test]
    fn release_sol_with_oracle_tip_account_count() {
        let pid = Pubkey::new_unique();
        let caller = Pubkey::new_unique();
        let seller = Pubkey::new_unique();
        let oracle = Pubkey::new_unique();
        let ix = build_release_payment_instruction(
            pid,
            caller,
            seller,
            Pubkey::default(),
            &[1u8; 32],
            Some(oracle),
        );
        // SOL path: 9 + 1 oracle = 10
        assert_eq!(ix.accounts.len(), 10);
    }

    #[test]
    fn refund_spl_no_oracle_tip_account_count() {
        let pid = Pubkey::new_unique();
        let caller = Pubkey::new_unique();
        let buyer = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let ix = build_refund_payment_instruction(pid, caller, buyer, mint, &[2u8; 32], None);
        // SPL path: 9 accounts + 0 optional = 9
        assert_eq!(ix.accounts.len(), 9);
        assert_eq!(ix.data, vec![SLAEscrowInstruction::RefundPayment as u8]);
    }

    #[test]
    fn refund_spl_with_oracle_tip_account_count() {
        let pid = Pubkey::new_unique();
        let caller = Pubkey::new_unique();
        let buyer = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let oracle = Pubkey::new_unique();
        let ix =
            build_refund_payment_instruction(pid, caller, buyer, mint, &[2u8; 32], Some(oracle));
        // SPL path: 9 + 4 (oracle_tokens, oracle_authority, ata_program, system_program) = 13
        assert_eq!(ix.accounts.len(), 13);
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
