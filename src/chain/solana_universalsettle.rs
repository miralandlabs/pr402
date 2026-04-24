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
pub const FEE_SHARD: &[u8] = b"fee_shard";
pub const FEE_SHARD_SOL: &[u8] = b"fee_shard_sol";

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
    AcceptAuthority = 108,
    CancelAuthorityProposal = 109,
    InitShard = 110,
    CollectFromShard = 111,
    UpdateShardConfig = 112,
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

/// UniversalSettle Config account body (matches `universalsettle/api/src/state/config.rs`, after the
/// 8-byte account discriminator).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Zeroable, bytemuck::Pod)]
pub struct Config {
    pub authority: Pubkey,
    pub fee_destination: Pubkey,
    pub updated_at: i64,
    pub min_fee_amount: u64,
    pub min_fee_amount_sol: u64,
    pub provisioning_fee_sol: u64,
    pub provisioning_fee_spl: u64,
    pub fee_bps: u16,
    pub discounted_fee_bps: u16,
    pub use_fee_shard: u8,
    pub shard_count: u8,
    pub _padding: [u8; 2],
}

/// UniversalSettle SplitVault account structure (matches universalsettle/api/src/state/split_vault.rs).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Zeroable, bytemuck::Pod)]
pub struct SplitVault {
    pub seller: Pubkey,
    pub sol_recovered: [u8; 8],
    pub spl_recovered: [u8; 8],
    pub is_provisioned: u8,
    pub bump: u8,
    pub is_sovereign: u8,
    pub _padding: [u8; 5],
}

/// Shard index for a seller (same rule as `universalsettle_api::state::derive_shard_index`).
pub fn derive_shard_index(seller: &Pubkey, shard_count: u8) -> u64 {
    if shard_count == 0 {
        return 0;
    }
    (seller.to_bytes()[0] % shard_count) as u64
}

pub fn fee_shard_pda(program_id: &Pubkey, index: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[FEE_SHARD, &index.to_le_bytes()], program_id)
}

pub fn fee_shard_sol_storage_pda(program_id: &Pubkey, shard_pda: Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[FEE_SHARD_SOL, shard_pda.as_ref()], program_id)
}

/// Fee leg destinations for [`build_sweep_instruction`].
///
/// When fee sharding is enabled (`use_fee_shard == 1` and `shard_count > 0`), protocol fees must be
/// credited to the seller’s shard SOL storage (native) or to the fee ATA owned by the shard PDA
/// (SPL), not directly to the treasury wallet.
///
/// Returns `(fee_sol_lamports_receiver, spl_fee_token_account_owner)`.
pub fn sweep_fee_destinations(
    program_id: &Pubkey,
    treasury: &Pubkey,
    seller: &Pubkey,
    use_fee_shard: u8,
    shard_count: u8,
) -> (Pubkey, Pubkey) {
    if use_fee_shard == 1 && shard_count > 0 {
        let idx = derive_shard_index(seller, shard_count);
        let (shard_pda, _) = fee_shard_pda(program_id, idx);
        let (shard_sol_storage, _) = fee_shard_sol_storage_pda(program_id, shard_pda);
        (shard_sol_storage, shard_pda)
    } else {
        (*treasury, *treasury)
    }
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
/// **`fee_sol_receiver`:** writable account that receives the facilitator’s native SOL fee (treasury
/// wallet when sharding is off, or **fee-shard SOL storage PDA** when on).
///
/// **`fee_token_owner`:** owner pubkey for the facilitator fee ATA when sweeping SPL (treasury when
/// sharding is off, or **fee-shard PDA** when on).
///
/// **`amount`:** pass **`0`** to sweep **all** available balance after rent (SOL) or full vault ATA
/// balance (SPL), matching on-chain `Sweep` semantics. Non-zero caps the sweep to that amount.
#[allow(clippy::too_many_arguments)]
pub fn build_sweep_instruction(
    program_id: Pubkey,
    payer: Pubkey,
    vault: Pubkey,
    seller: Pubkey,
    fee_sol_receiver: Pubkey,
    fee_token_owner: Pubkey,
    token_mint: Pubkey,
    amount: u64,
    is_sol: bool,
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
        accounts.push(AccountMeta::new(fee_sol_receiver, false));
        accounts.push(AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false));
    } else {
        let token_program = spl_token_program.unwrap_or(spl_token::id());
        let vault_tokens = associated_token_address(&vault, &token_mint, &token_program);
        let seller_tokens = associated_token_address(&seller, &token_mint, &token_program);
        let fee_dest_tokens =
            associated_token_address(&fee_token_owner, &token_mint, &token_program);

        accounts.push(AccountMeta::new(vault_tokens, false));
        accounts.push(AccountMeta::new(seller_tokens, false));
        accounts.push(AccountMeta::new(fee_dest_tokens, false));
        accounts.push(AccountMeta::new_readonly(token_mint, false));
        accounts.push(AccountMeta::new_readonly(token_program, false));
    }

    Instruction {
        program_id,
        accounts,
        data: instruction_data,
    }
}

/// Resolve the UniversalSettle payment destination for quoting or display (legacy SPL Token for SPL).
pub fn get_payment_address(vault: &Pubkey, mint: Option<&Pubkey>, program_id: &Pubkey) -> Pubkey {
    get_payment_address_with_token_program(vault, mint, program_id, None)
}

/// Same as [`get_payment_address`], but pass `Some(token_program_id)` for Token-2022 vault ATAs.
pub fn get_payment_address_with_token_program(
    vault: &Pubkey,
    mint: Option<&Pubkey>,
    program_id: &Pubkey,
    spl_token_program: Option<&Pubkey>,
) -> Pubkey {
    match mint {
        Some(mint) => {
            let tp = spl_token_program.copied().unwrap_or(spl_token::id());
            associated_token_address(vault, mint, &tp)
        }
        None => derive_sol_storage_pda(vault, program_id).0,
    }
}

pub fn derive_config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[CONFIG], program_id)
}

pub fn derive_sol_storage_pda(vault_pda: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SOL_STORAGE, vault_pda.as_ref()], program_id)
}
