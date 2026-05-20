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
use crate::chain::TxBudget;
use crate::db::Pr402Db;
use crate::util::tx_builder::{
    associated_token_address, compute_budget_ix_set_limit, compute_budget_ix_set_price,
    create_associated_token_account_idempotent_ix, estimate_blockhash_expiry_unix,
};

/// Canonical “virtual mint” for native SOL in x402 / pr402 (matches payment `asset` conventions).
pub const NATIVE_SOL_ASSET_MINT: Pubkey = pubkey!("11111111111111111111111111111111");

/// Canonical USDC mint on Solana mainnet-beta (Circle USDC).
pub const USDC_MAINNET: Pubkey = pubkey!("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
/// Canonical USDC mint on Solana devnet (Circle devnet faucet).
pub const USDC_DEVNET: Pubkey = pubkey!("4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU");
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
    /// Friendly label, e.g. `SOL`, `USDC`, `USDT`, or `spl` for an arbitrary mint.
    pub asset: String,
    /// SPL mint base58; for native SOL, [`NATIVE_SOL_ASSET_MINT`] (all-ones pubkey per convention).
    pub asset_mint: String,
    pub vault_pda: String,
    pub sol_storage_pda: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_token_ata: Option<String>,
    pub already_provisioned: bool,
    /// Machine-readable summary of what this call will do on-chain (or what it observed).
    /// One of:
    ///   - `"ALREADY_PROVISIONED"` — both vault and (if applicable) ATA exist; `transaction` is absent.
    ///   - `"VAULT_AND_ATA"` — first-time SPL provisioning (two setup ixs).
    ///   - `"VAULT_ONLY"` — first-time native-SOL provisioning (single `CreateVault` ix).
    ///   - `"ATA_ONLY"` — vault exists, adding a new SPL mint (single ATA-create ix).
    ///
    /// Clients should prefer this over parsing `notes[]`. Backward-compatible: the field is
    /// additive and absent in older clients; `already_provisioned` still carries the same
    /// boolean signal.
    pub status_code: String,
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

/// Canonical USDC mint for a given cluster. `None` for testnet (no
/// canonical USDC). Used by the `/onboard` preview to pre-compute the
/// per-mint escrow PDAs callers will most often want.
pub fn canonical_usdc_mint(provider: &SolanaChainProvider) -> Option<Pubkey> {
    if cluster_is_devnet(provider) {
        Some(USDC_DEVNET)
    } else {
        let id = provider.chain_id().to_string().to_lowercase();
        if id.contains("testnet") {
            None
        } else {
            Some(USDC_MAINNET)
        }
    }
}

/// Friendly suffix to label the cluster's USDC entry in
/// `/onboard.schemes[*].vaultPdaPreviews`. Mainnet returns just
/// `"USDC"`, devnet returns `"USDC (devnet)"`.
pub fn canonical_usdc_label(provider: &SolanaChainProvider) -> &'static str {
    if cluster_is_devnet(provider) {
        "USDC (devnet)"
    } else {
        "USDC"
    }
}

/// Resolve human-friendly asset names or a raw mint pubkey.
///
/// Supported friendly labels: `SOL`, `USDC`, `USDT`. Raw base58 SPL mints are accepted
/// if the operator's `PR402_ALLOWED_PAYMENT_MINTS` allowlist includes them.
///
/// **WSOL is deliberately not supported** as a friendly label. The UniversalSettle
/// on-chain program keeps **two** minimum-fee values — `min_fee_amount` (for SPL,
/// assumed 6-decimals stablecoin) and `min_fee_amount_sol` (for native SOL, 9
/// decimals) — and picks one via the `is_sol` flag on the Sweep instruction. WSOL
/// is an SPL token with 9 decimals, so it would settle against the 6-decimal SPL
/// floor, producing a ~11× undercharge at the floor boundary. Until the program
/// supports per-mint floors, operators should keep WSOL out of the allowlist and
/// sellers should use native `SOL` instead.
pub fn resolve_seller_asset(
    asset: &str,
    devnet: bool,
) -> Result<ResolvedSellerAsset, SellerProvisionError> {
    let a = asset.trim();
    if a.is_empty() {
        return Err(SellerProvisionError::InvalidInput(
            "asset is required (e.g. SOL, USDC, USDT, or a base58 mint address)".into(),
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
        "wsol" | "wrapped_sol" | "wrapped-sol" => Err(SellerProvisionError::InvalidInput(
            "WSOL is not a supported seller rail on this facilitator. \
             Use `SOL` (native) for SOL-denominated payments, or `USDC` for stable-unit payments. \
             (WSOL would settle against the 6-decimal SPL fee floor but has 9 decimals; until \
             per-mint floors are supported, the rail is disabled to avoid silently undercharging \
             sellers.)"
                .into(),
        )),
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
            "Unknown asset {:?}; use SOL, USDC, USDT, or a base58 mint address",
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
            status_code: "ALREADY_PROVISIONED".to_string(),
            transaction: None,
            recent_blockhash: None,
            recent_blockhash_expires_at: None,
            fee_payer: seller.to_string(),
            payer: seller.to_string(),
            payer_signature_index: 0,
            notes,
        });
    }

    // Pick budget based on the actual instruction set being submitted.
    // setup_ixs can be 1 or 2 instructions at this point (0 was early-returned above).
    //   2 → CreateVault + ATA (first-time SPL provisioning, e.g. USDC)
    //   1 + !vault_exists → CreateVault only (first-time SOL provisioning)
    //   1 + vault_exists  → ATA only (adding a new SPL mint to an existing vault)
    let budget = match setup_ixs.len() {
        2 => TxBudget::VaultCreateWithAta,
        _ if !vault_exists => TxBudget::VaultCreate,
        _ => TxBudget::VaultAtaCreate,
    };
    let cu_limit = compute_budget_ix_set_limit(budget.cu_limit());
    let cu_price = compute_budget_ix_set_price(budget.cu_price());
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

    // The `budget` branch above enumerates exactly the 3 non-idempotent shapes. Mirror them in
    // the machine-readable `statusCode` so UIs don't have to infer from `notes[]`.
    let status_code = match budget {
        TxBudget::VaultCreateWithAta => "VAULT_AND_ATA",
        TxBudget::VaultCreate => "VAULT_ONLY",
        TxBudget::VaultAtaCreate => "ATA_ONLY",
        _ => "VAULT_ONLY", // Unreachable under current budget mapping; safe default.
    }
    .to_string();

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
        status_code,
        transaction: Some(tx_b64),
        recent_blockhash: Some(blockhash.to_string()),
        recent_blockhash_expires_at: Some(estimate_blockhash_expiry_unix()),
        fee_payer: seller.to_string(),
        payer: seller.to_string(),
        payer_signature_index: 0,
        notes,
    })
}
