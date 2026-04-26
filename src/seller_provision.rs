//! UniversalSettle seller provisioning: **SplitVault** (`CreateVault`) plus the payment surface for a
//! chosen asset — native SOL storage (created with the vault) or an SPL **vault ATA** for a mint.
//!
//! Same `(wallet, asset)` request is **idempotent**: if the vault and (when applicable) ATA already
//! exist, the response has `alreadyProvisioned: true` and no `transaction`.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use solana_pubkey::pubkey;
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;
use std::str::FromStr;

use crate::chain::solana::{SolanaChainProvider, TOKEN_2022_PROGRAM_ID};
use crate::chain::solana_universalsettle::build_create_vault_instruction;
use crate::db::Pr402Db;
use crate::util::tx_builder::{
    associated_token_address, compute_budget_ix_set_limit, compute_budget_ix_set_price,
    create_associated_token_account_idempotent_ix, estimate_blockhash_expiry_unix,
};

/// Canonical “virtual mint” for native SOL in x402 / pr402 (matches payment `asset` conventions).
pub const NATIVE_SOL_ASSET_MINT: Pubkey = pubkey!("11111111111111111111111111111111");

const WSOL_MINT: Pubkey = pubkey!("So11111111111111111111111111111111111111112");
const USDC_MAINNET: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const USDC_DEVNET: Pubkey = pubkey!("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");
const USDT_MAINNET: Pubkey = pubkey!("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");

#[derive(Debug, Clone)]
pub enum ResolvedSellerAsset {
    NativeSol,
    Spl { mint: Pubkey, label: &'static str },
}

#[derive(Debug, thiserror::Error)]
pub enum SellerProvisionError {
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    MintNotAllowed(String),
    #[error("{0}")]
    Chain(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SellerProvisionTxResponse {
    pub schema_version: String,
    pub wallet: String,
    /// Friendly label, e.g. `SOL`, `USDC`, `WSOL`, `USDT`, or `spl` for an arbitrary mint.
    pub asset: String,
    /// SPL mint base58; for native SOL, [`NATIVE_SOL_ASSET_MINT`] (all-ones pubkey per convention).
    pub asset_mint: String,
    pub vault_pda: String,
    pub sol_storage_pda: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_token_ata: Option<String>,
    pub already_provisioned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_blockhash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_blockhash_expires_at: Option<u64>,
    pub fee_payer: String,
    pub payer: String,
    pub payer_signature_index: usize,
    pub notes: Vec<String>,
}

pub fn cluster_is_devnet(provider: &SolanaChainProvider) -> bool {
    provider.rpc_url().contains("devnet")
        || provider
            .chain_id()
            .to_string()
            .to_lowercase()
            .contains("devnet")
}

/// Resolve human-friendly asset names or a raw mint pubkey.
pub fn resolve_seller_asset(
    asset: &str,
    devnet: bool,
) -> Result<ResolvedSellerAsset, SellerProvisionError> {
    let a = asset.trim();
    if a.is_empty() {
        return Err(SellerProvisionError::InvalidInput(
            "asset is required (e.g. SOL, USDC, WSOL, USDT, or a mint address)".into(),
        ));
    }
    if let Ok(pk) = Pubkey::from_str(a) {
        if pk == Pubkey::default() || pk == NATIVE_SOL_ASSET_MINT {
            return Ok(ResolvedSellerAsset::NativeSol);
        }
        return Ok(ResolvedSellerAsset::Spl {
            mint: pk,
            label: "spl",
        });
    }
    match a.to_ascii_lowercase().as_str() {
        "sol" | "native" | "native_sol" | "lamports" => Ok(ResolvedSellerAsset::NativeSol),
        "wsol" | "wrapped_sol" | "wrapped-sol" => Ok(ResolvedSellerAsset::Spl {
            mint: WSOL_MINT,
            label: "WSOL",
        }),
        "usdc" => Ok(ResolvedSellerAsset::Spl {
            mint: if devnet { USDC_DEVNET } else { USDC_MAINNET },
            label: "USDC",
        }),
        "usdt" => {
            if devnet {
                Err(SellerProvisionError::InvalidInput(
                    "USDT: use the SPL mint address on devnet".into(),
                ))
            } else {
                Ok(ResolvedSellerAsset::Spl {
                    mint: USDT_MAINNET,
                    label: "USDT",
                })
            }
        }
        _ => Err(SellerProvisionError::InvalidInput(format!(
            "Unknown asset {:?}; use SOL, USDC, USDT, WSOL, or a base58 mint address",
            a
        ))),
    }
}

/// Maps a resolved seller asset to `resource_providers.settlement_mode` / `spl_mint`.
pub fn resolved_seller_asset_to_settlement_rail(
    resolved: &ResolvedSellerAsset,
) -> (&'static str, Option<String>) {
    match resolved {
        ResolvedSellerAsset::NativeSol => ("native_sol", None),
        ResolvedSellerAsset::Spl { mint, .. } => ("spl", Some(mint.to_string())),
    }
}

pub async fn build_universalsettle_seller_provision(
    provider: &SolanaChainProvider,
    db: Option<&Pr402Db>,
    wallet: &str,
    asset: &str,
) -> Result<SellerProvisionTxResponse, SellerProvisionError> {
    let us = provider
        .universalsettle()
        .ok_or_else(|| SellerProvisionError::Chain("UniversalSettle is not configured".into()))?;
    let seller = Pubkey::from_str(wallet)
        .map_err(|e| SellerProvisionError::InvalidInput(format!("wallet: {}", e)))?;
    let devnet = cluster_is_devnet(provider);
    let resolved = resolve_seller_asset(asset, devnet)?;

    let mint_for_allowlist = match &resolved {
        ResolvedSellerAsset::NativeSol => NATIVE_SOL_ASSET_MINT,
        ResolvedSellerAsset::Spl { mint, .. } => *mint,
    };
    crate::parameters::ensure_allowed_payment_mint(db, &mint_for_allowlist)
        .await
        .map_err(SellerProvisionError::MintNotAllowed)?;

    let (reg_mode, reg_mint_owned) = resolved_seller_asset_to_settlement_rail(&resolved);
    if let Some(db) = db {
        db.assert_merchant_single_rail_policy(wallet, reg_mode, reg_mint_owned.as_deref())
            .await
            .map_err(|e| SellerProvisionError::InvalidInput(e.to_string()))?;
    }

    let program_id = us.program_id;
    let (vault_pda, _) = provider.get_vault_pda(&seller);
    let (sol_storage, _) = provider.get_sol_storage_pda(vault_pda);

    let vault_exists = provider
        .account_exists(&vault_pda)
        .await
        .map_err(|e| SellerProvisionError::Chain(e.to_string()))?;

    let mut setup_ixs: Vec<solana_transaction::Instruction> = Vec::new();
    let mut notes: Vec<String> = Vec::new();

    let (friendly_asset, asset_mint_json, vault_token_ata_pubkey): (
        String,
        String,
        Option<Pubkey>,
    ) = match &resolved {
        ResolvedSellerAsset::NativeSol => {
            if !vault_exists {
                setup_ixs.push(build_create_vault_instruction(program_id, seller, seller));
                notes.push(
                    "This transaction creates your SplitVault and native SOL storage PDA.".into(),
                );
            } else {
                notes.push("SplitVault and SOL storage already exist.".into());
            }
            ("SOL".to_string(), NATIVE_SOL_ASSET_MINT.to_string(), None)
        }
        ResolvedSellerAsset::Spl { mint, label } => {
            let mint_acc = provider
                .rpc_client()
                .get_account(mint)
                .await
                .map_err(|e| SellerProvisionError::Chain(format!("mint lookup: {}", e)))?;
            let token_program = if mint_acc.owner == spl_token::id() {
                spl_token::id()
            } else if mint_acc.owner == TOKEN_2022_PROGRAM_ID {
                TOKEN_2022_PROGRAM_ID
            } else {
                return Err(SellerProvisionError::InvalidInput(format!(
                    "mint {} owner is not SPL Token or Token-2022",
                    mint
                )));
            };
            let ata = associated_token_address(&vault_pda, mint, &token_program);
            let ata_exists = provider
                .account_exists(&ata)
                .await
                .map_err(|e| SellerProvisionError::Chain(e.to_string()))?;

            if !vault_exists {
                setup_ixs.push(build_create_vault_instruction(program_id, seller, seller));
            }
            if !ata_exists {
                setup_ixs.push(create_associated_token_account_idempotent_ix(
                    &seller,
                    &vault_pda,
                    mint,
                    &token_program,
                ));
            }

            if vault_exists && ata_exists {
                notes.push("SplitVault and vault token ATA already exist for this mint.".into());
            } else if !vault_exists && !ata_exists {
                notes.push("Creates SplitVault, SOL storage, and vault ATA for this token.".into());
            } else if !vault_exists {
                notes.push("Creates SplitVault and vault ATA.".into());
            } else {
                notes.push("Creates vault ATA for this SPL mint.".into());
            }

            (label.to_string(), mint.to_string(), Some(ata))
        }
    };

    let vault_token_ata_str = vault_token_ata_pubkey.map(|p| p.to_string());
    let already_provisioned = setup_ixs.is_empty();

    if already_provisioned {
        return Ok(SellerProvisionTxResponse {
            schema_version: "1.0.0".to_string(),
            wallet: wallet.to_string(),
            asset: friendly_asset,
            asset_mint: asset_mint_json,
            vault_pda: vault_pda.to_string(),
            sol_storage_pda: sol_storage.to_string(),
            vault_token_ata: vault_token_ata_str,
            already_provisioned: true,
            transaction: None,
            recent_blockhash: None,
            recent_blockhash_expires_at: None,
            fee_payer: seller.to_string(),
            payer: seller.to_string(),
            payer_signature_index: 0,
            notes,
        });
    }

    let cu_limit = compute_budget_ix_set_limit(provider.max_compute_unit_limit());
    let cu_price = compute_budget_ix_set_price(provider.max_compute_unit_price());
    let mut ixs = vec![cu_limit, cu_price];
    ixs.extend(setup_ixs);

    let blockhash = provider
        .rpc_client()
        .get_latest_blockhash()
        .await
        .map_err(|e| SellerProvisionError::Chain(e.to_string()))?;

    let message = solana_message::v0::Message::try_compile(&seller, &ixs, &[], blockhash)
        .map_err(|e| SellerProvisionError::Chain(e.to_string()))?;
    let tx = VersionedTransaction {
        signatures: vec![solana_signature::Signature::default()],
        message: solana_message::VersionedMessage::V0(message),
    };
    let tx_b64 = STANDARD
        .encode(bincode::serialize(&tx).map_err(|e| SellerProvisionError::Chain(e.to_string()))?);

    notes.push("Sign and send with the seller wallet (sovereign provisioning).".into());

    Ok(SellerProvisionTxResponse {
        schema_version: "1.0.0".to_string(),
        wallet: wallet.to_string(),
        asset: friendly_asset,
        asset_mint: asset_mint_json,
        vault_pda: vault_pda.to_string(),
        sol_storage_pda: sol_storage.to_string(),
        vault_token_ata: vault_token_ata_str,
        already_provisioned: false,
        transaction: Some(tx_b64),
        recent_blockhash: Some(blockhash.to_string()),
        recent_blockhash_expires_at: Some(estimate_blockhash_expiry_unix()),
        fee_payer: seller.to_string(),
        payer: seller.to_string(),
        payer_signature_index: 0,
        notes,
    })
}
