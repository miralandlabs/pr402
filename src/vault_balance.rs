//! Read UniversalSettle SplitVault balances via RPC (no transaction fees).
//!
//! Mirrors `spl-token-balance-serverless`: derive the vault ATA (owner = SplitVault PDA) and use
//! [`RpcClient::get_account`]. For a wallet-held balance, Solana’s
//! `getTokenAccountsByOwner` is also available on [`RpcClient`] if you need to scan all mints.

use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_client::rpc_response::UiAccountData;
use solana_pubkey::Pubkey;
use std::str::FromStr;

use crate::chain::solana::{ASSOCIATED_TOKEN_PROGRAM_ID, TOKEN_PROGRAM_ID};

const VAULT_SEED: &[u8] = b"vault";
const SOL_STORAGE_SEED: &[u8] = b"sol_storage";

/// Snapshot of spendable balances for one seller's UniversalSettle vaults.
#[derive(Debug, Clone)]
pub struct UniversalSettleVaultSnapshot {
    pub seller: Pubkey,
    pub program_id: Pubkey,
    pub split_vault_pda: Pubkey,
    pub vault_sol_storage_pda: Pubkey,
    /// Lamports the sweep can move (storage lamports − rent exempt minimum for empty account).
    pub spendable_lamports: u64,
    pub vault_spl_ata: Option<Pubkey>,
    /// Raw token amount in vault ATA (0 if mint not queried or ATA missing).
    pub spl_amount_raw: u64,
    pub spl_decimals: Option<u8>,
}

pub fn split_vault_pda(program_id: &Pubkey, seller: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[VAULT_SEED, seller.as_ref()], program_id)
}

pub fn vault_sol_storage_pda(program_id: &Pubkey, vault: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[SOL_STORAGE_SEED, vault.as_ref()], program_id)
}

/// SPL ATA owned by `vault` for `mint` (Token or Token-2022 per `token_program`).
pub fn vault_spl_ata(vault: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[vault.as_ref(), token_program.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
    .0
}

/// Token account amount (raw u64) and decimals from mint account bytes when needed.
fn spl_token_amount_from_account_data(data: &[u8]) -> Option<u64> {
    if data.len() < 72 {
        return None;
    }
    let mut amt = [0u8; 8];
    amt.copy_from_slice(&data[64..72]);
    Some(u64::from_le_bytes(amt))
}

fn mint_decimals(data: &[u8]) -> Option<u8> {
    // SPL Mint layout: decimals at byte index 44 (82-byte account).
    if data.len() < 45 {
        return None;
    }
    Some(data[44])
}

/// SPL balance for `owner` and `mint` via `getTokenAccountsByOwner` + mint filter (jsonParsed).
/// Same RPC pattern as `spl-token-balance-serverless` (wallet-held balance; no client-side ATA derivation required).
pub async fn fetch_spl_raw_balance_by_owner_and_mint(
    rpc: &RpcClient,
    owner: &Pubkey,
    mint: &Pubkey,
) -> Result<(u64, Option<u8>), String> {
    let accounts = rpc
        .get_token_accounts_by_owner(owner, TokenAccountsFilter::Mint(*mint))
        .await
        .map_err(|e| e.to_string())?;

    let mut total: u64 = 0;
    let mut decimals: Option<u8> = None;

    for keyed in accounts {
        if let UiAccountData::Json(parsed) = keyed.account.data {
            if let Some(info) = parsed.parsed.get("info") {
                if let Some(tok) = info.get("tokenAmount") {
                    if let Some(amt) = tok.get("amount").and_then(|a| a.as_str()) {
                        total = total.saturating_add(amt.parse::<u64>().unwrap_or(0));
                    }
                    if decimals.is_none() {
                        decimals = tok
                            .get("decimals")
                            .and_then(|d| d.as_u64())
                            .map(|d| d as u8);
                    }
                }
            }
        }
    }

    if decimals.is_none() {
        decimals = rpc
            .get_account(mint)
            .await
            .ok()
            .and_then(|mint_acc| mint_decimals(&mint_acc.data));
    }

    Ok((total, decimals))
}

/// Fetch vault SOL + optional SPL balance for UniversalSettle (RPC reads only).
///
/// When `spl_mint` is set, `spl_token_program` selects the ATA domain (`None` → legacy Token program).
pub async fn fetch_universalsettle_vault_snapshot(
    rpc: &RpcClient,
    program_id: Pubkey,
    seller: Pubkey,
    spl_mint: Option<Pubkey>,
    spl_token_program: Option<Pubkey>,
) -> Result<UniversalSettleVaultSnapshot, String> {
    let (split_vault_pda, _) = split_vault_pda(&program_id, &seller);
    let (vault_sol_storage_pda, _) = vault_sol_storage_pda(&program_id, &split_vault_pda);

    let rent_min = rpc
        .get_minimum_balance_for_rent_exemption(0)
        .await
        .map_err(|e| e.to_string())?;

    let storage_lamports = match rpc.get_account(&vault_sol_storage_pda).await {
        Ok(acc) => acc.lamports,
        Err(_) => 0,
    };
    let spendable_lamports = storage_lamports.saturating_sub(rent_min);

    let (vault_spl_ata, spl_amount_raw) = if let Some(mint) = spl_mint {
        let token_program = spl_token_program.as_ref().unwrap_or(&TOKEN_PROGRAM_ID);
        let ata = vault_spl_ata(&split_vault_pda, &mint, token_program);
        match rpc.get_account(&ata).await {
            Ok(acc) => {
                let raw = spl_token_amount_from_account_data(&acc.data).unwrap_or(0);
                (Some(ata), raw)
            }
            Err(_) => (Some(ata), 0),
        }
    } else {
        (None, 0)
    };

    let spl_decimals = if let Some(mint) = spl_mint {
        rpc.get_account(&mint)
            .await
            .ok()
            .and_then(|mint_acc| mint_decimals(&mint_acc.data))
    } else {
        None
    };

    Ok(UniversalSettleVaultSnapshot {
        seller,
        program_id,
        split_vault_pda,
        vault_sol_storage_pda,
        spendable_lamports,
        vault_spl_ata,
        spl_amount_raw,
        spl_decimals,
    })
}

/// Parse seller pubkey from base58 string.
pub fn parse_seller(s: &str) -> Result<Pubkey, String> {
    Pubkey::from_str(s).map_err(|_| format!("invalid seller pubkey: {}", s))
}
