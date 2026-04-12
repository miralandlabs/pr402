//! UniversalSettle instruction building for Solana 3.x.
//!
//! This module manually builds UniversalSettle instructions compatible with the
//! steel framework's discriminator format, without depending on Solana 2.x SDKs.

use solana_pubkey::{pubkey, Pubkey};
use solana_transaction::{AccountMeta, Instruction};

use super::solana::SYSTEM_PROGRAM_ID;

/// SPL Associated Token Account program (`spl_associated_token_account::ID`).
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey = pubkey!("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub const VAULT: &[u8] = b"vault";
pub const SOL_STORAGE: &[u8] = b"sol_storage";
pub const CONFIG: &[u8] = b"config";

fn associated_token_address(wallet: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            &wallet.to_bytes(),
            &token_program.to_bytes(),
            &mint.to_bytes(),
        ],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

/// UniversalSettle instruction discriminators (matches universalsettle/api/src/instruction.rs).
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UniversalSettleInstruction {
    Sweep = 0,
    CreateVault = 1,
    Initialize = 100,
    UpdateAuthority = 101,
    UpdateFeeRate = 102,
    UpdateFeeDestination = 103,
    UpdateMinFeeAmount = 104,
    UpdateMinFeeAmountSol = 105,
    UpdateProvisioningFee = 106,
    UpdateDiscountedFeeRate = 107,
}

/// UniversalSettle CreateVault instruction data structure.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CreateVaultData {
    pub seller: Pubkey,
}

impl CreateVaultData {
    pub fn to_bytes(&self) -> Vec<u8> {
        self.seller.to_bytes().to_vec()
    }
}

/// UniversalSettle Sweep instruction data structure.
/// `amount` encodes little-endian u64: **`0` means sweep all spendable balance** on-chain (see
/// `x402/universalsettle` `process_sweep`); non-zero sweeps at most that amount.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SweepData {
    pub token_mint: Pubkey,
    pub amount: [u8; 8],
    pub is_sol: [u8; 1],
    pub _padding: [u8; 7],
}

impl SweepData {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(48);
        bytes.extend_from_slice(&self.token_mint.to_bytes());
        bytes.extend_from_slice(&self.amount);
        bytes.extend_from_slice(&self.is_sol);
        bytes.extend_from_slice(&self._padding);
        bytes
    }
}

/// UniversalSettle Config account structure (matches universalsettle/api/src/state/config.rs).
/// Corrected to 112-byte layout for v0.1.3.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Zeroable, bytemuck::Pod)]
pub struct Config {
    pub authority: Pubkey,             // 32
    pub fee_destination: Pubkey,       // 32
    pub updated_at: [u8; 8],           // 8
    pub min_fee_amount: [u8; 8],       // 8
    pub min_fee_amount_sol: [u8; 8],   // 8
    pub provisioning_fee_sol: [u8; 8], // 8
    pub provisioning_fee_spl: [u8; 8], // 8
    pub fee_bps: [u8; 2],              // 2
    pub discounted_fee_bps: [u8; 2],   // 2
    pub _padding: [u8; 4],             // 4
}

/// UniversalSettle SplitVault account structure (matches universalsettle/api/src/state/split_vault.rs).
/// Corrected to 56-byte layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Zeroable, bytemuck::Pod)]
pub struct SplitVault {
    pub seller: Pubkey,         // 32
    pub sol_recovered: [u8; 8], // 8
    pub spl_recovered: [u8; 8], // 8
    pub is_provisioned: u8,     // 1
    pub bump: u8,               // 1
    pub is_sovereign: u8,       // 1
    pub _padding: [u8; 5],      // 5
}

pub fn build_create_vault_instruction(
    program_id: Pubkey,
    payer: Pubkey,
    seller: Pubkey,
) -> Instruction {
    let (vault_pda, _) = Pubkey::find_program_address(&[VAULT, seller.as_ref()], &program_id);
    let (vault_sol_storage, _) = derive_sol_storage_pda(&vault_pda, &program_id);

    let data = CreateVaultData { seller };

    let mut instruction_data = Vec::with_capacity(33);
    instruction_data.push(UniversalSettleInstruction::CreateVault as u8);
    instruction_data.extend_from_slice(&data.to_bytes());

    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(vault_sol_storage, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
        data: instruction_data,
    }
}

/// `spl_token_program` must match the rail used for payment (legacy Token vs Token-2022) when `!is_sol`.
///
/// **`amount`:** pass **`0`** to sweep **all** available balance after rent (SOL) or full vault ATA
/// balance (SPL), matching on-chain `Sweep` semantics. Non-zero caps the sweep to that amount.
#[allow(clippy::too_many_arguments)]
pub fn build_sweep_instruction(
    program_id: Pubkey,
    payer: Pubkey,
    vault: Pubkey,
    seller: Pubkey,
    fee_destination: Pubkey,
    token_mint: Pubkey,
    amount: u64,
    is_sol: bool,
    // SPL program for vault/seller/fee ATAs when `!is_sol`; `None` uses legacy Token program.
    spl_token_program: Option<Pubkey>,
) -> Instruction {
    let (config_pda, _) = derive_config_pda(&program_id);
    let data = SweepData {
        token_mint,
        amount: amount.to_le_bytes(),
        is_sol: if is_sol { [1] } else { [0] },
        _padding: [0; 7],
    };

    let mut instruction_data = Vec::with_capacity(49);
    instruction_data.push(UniversalSettleInstruction::Sweep as u8);
    instruction_data.extend_from_slice(&data.to_bytes());

    let mut accounts = vec![
        AccountMeta::new(payer, true),
        AccountMeta::new(vault, false),
        AccountMeta::new_readonly(config_pda, false),
    ];

    if is_sol {
        let (vault_sol_storage, _) = derive_sol_storage_pda(&vault, &program_id);
        accounts.push(AccountMeta::new(vault_sol_storage, false));
        accounts.push(AccountMeta::new(seller, false));
        accounts.push(AccountMeta::new(fee_destination, false));
        accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
    } else {
        let token_program = spl_token_program.unwrap_or(spl_token::id());
        let vault_tokens = associated_token_address(&vault, &token_mint, &token_program);
        let seller_tokens = associated_token_address(&seller, &token_mint, &token_program);
        let fee_dest_tokens =
            associated_token_address(&fee_destination, &token_mint, &token_program);

        accounts.push(AccountMeta::new(vault_tokens, false));
        accounts.push(AccountMeta::new(seller_tokens, false));
        accounts.push(AccountMeta::new(fee_dest_tokens, false));
        accounts.push(AccountMeta::new_readonly(token_program, false));
    }

    Instruction {
        program_id,
        accounts,
        data: instruction_data,
    }
}

pub fn get_payment_address(vault: &Pubkey, mint: Option<&Pubkey>, program_id: &Pubkey) -> Pubkey {
    match mint {
        Some(mint) => associated_token_address(vault, mint, &spl_token::id()),
        None => derive_sol_storage_pda(vault, program_id).0,
    }
}

pub fn derive_config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CONFIG], program_id)
}

pub fn derive_sol_storage_pda(vault_pda: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SOL_STORAGE, vault_pda.as_ref()], program_id)
}
