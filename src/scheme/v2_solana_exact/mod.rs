//! v2:solana:exact payment scheme implementation.

pub mod types;

use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;
use base64::{engine::general_purpose::STANDARD, Engine};

use crate::chain::solana::SolanaChainProvider;
use crate::chain::ChainProvider;
use crate::proto;
use crate::proto::PaymentVerificationError;
use crate::scheme::v2_solana_exact::types::ExactScheme;
use crate::scheme::v2_solana_exact::types::SupportedPaymentKindExtra;
use crate::scheme::{
    X402SchemeFacilitator, X402SchemeFacilitatorBuilder, X402SchemeFacilitatorError, X402SchemeId,
};

// Re-export shared Solana verification logic
pub mod shared;
use crate::chain::solana_universalsettle::{Config as USConfig, SplitVault};
use shared::{
    settle_transaction, verify_transaction as shared_verify_transaction, TransferRequirement,
    VerifyTransferResult,
};

pub struct V2SolanaExact;

impl X402SchemeId for V2SolanaExact {
    fn namespace(&self) -> &str {
        "solana"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}

impl X402SchemeFacilitatorBuilder for V2SolanaExact {
    fn build(
        &self,
        provider: ChainProvider,
        _config: Option<serde_json::Value>,
        db: Option<crate::db::Pr402Db>,
    ) -> Result<Box<dyn X402SchemeFacilitator>, Box<dyn Error>> {
        Ok(Box::new(V2SolanaExactFacilitator {
            provider: provider.solana,
            db,
        }))
    }
}

pub struct V2SolanaExactFacilitator {
    provider: Arc<SolanaChainProvider>,
    db: Option<crate::db::Pr402Db>,
}

#[async_trait::async_trait]
impl X402SchemeFacilitator for V2SolanaExactFacilitator {
    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, X402SchemeFacilitatorError> {
        let request = types::VerifyRequest::from_proto(request.clone())?;
        self.validate_mint(request.payment_requirements.asset.pubkey())
            .await?;
        let verification = verify_transfer(&self.provider, &request).await?;
        Ok(proto::v2::VerifyResponse::valid(verification.payer.to_string()).into())
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeFacilitatorError> {
        let request = types::SettleRequest::from_proto(request.clone())?;
        self.validate_mint(request.payment_requirements.asset.pubkey())
            .await?;
        let verification = verify_transfer(&self.provider, &request).await?;

        // REFINEMENT: Extract authoritative identity for JIT provisioning and final sweep destination.
        let merchant_id = verification.merchant_identity;
        let final_beneficiary = verification.final_beneficiary;
        let seller = merchant_id;

        // Just-in-Time Provisioning: Ensure vault is created only when a real settlement is requested.
        if let Some(us_config) = self.provider.universalsettle() {
            let fee_dest = us_config.fee_destination.ok_or_else(|| {
                X402SchemeFacilitatorError::OnchainFailure(
                    "UniversalSettle fee destination not configured".to_string(),
                )
            })?;
            let fee_bps = us_config.fee_bps.unwrap_or(100);
            let asset = *request.payment_requirements.asset.pubkey();
            self.provider
                .ensure_vault_setup(seller.pubkey(), &fee_dest, fee_bps, Some(asset))
                .await?;
        }

        let payer = verification.payer.to_string();

        let tx_sig = settle_transaction(
            &self.provider,
            verification,
            Some(*final_beneficiary.pubkey()),
        )
        .await?;
        Ok(proto::v2::SettleResponse::Success {
            payer,
            transaction: tx_sig.to_string(),
            network: self.provider.chain_id().to_string(),
        }
        .into())
    }

    async fn supported(&self) -> Result<proto::SupportedResponse, X402SchemeFacilitatorError> {
        let chain_id = self.provider.chain_id();
        let kinds: Vec<proto::SupportedPaymentKind> = {
            let provider = &self.provider;
            let extra = if let Some(us_config) = provider.universalsettle() {
                let (config_address, _) = provider.get_config_pda(&us_config.program_id);
                let extra = SupportedPaymentKindExtra {
                    fee_payer: provider.fee_payer().into(),
                    program_id: us_config.program_id.into(),
                    config_address: config_address.into(),
                    fee_bps: us_config.fee_bps.unwrap_or(0).into(),
                    min_fee_amount: us_config.min_fee_amount.unwrap_or(0).into(),
                    min_fee_amount_sol: us_config.min_fee_amount_sol.unwrap_or(0).into(),
                    merchant_wallet: None,
                    beneficiary: None,
                };
                Some(serde_json::to_value(extra).unwrap())
            } else {
                // If UniversalSettle is not enabled, this scheme is technically legacy/direct transfer
                None
            };

            vec![proto::SupportedPaymentKind {
                x402_version: 2,
                scheme: ExactScheme.to_string(),
                network: chain_id.to_string(),
                extra,
            }]
        };
        let signers = {
            let mut signers = HashMap::with_capacity(1);
            signers.insert(
                chain_id,
                self.provider
                    .signer_addresses()
                    .iter()
                    .map(|a| a.to_string())
                    .collect(),
            );
            signers
        };
        Ok(proto::SupportedResponse {
            kinds,
            extensions: Vec::new(),
            signers,
        })
    }

    async fn onboard(
        &self,
        wallet: &str,
    ) -> Result<crate::facilitator::SchemeOnboardInfo, X402SchemeFacilitatorError> {
        let seller = solana_pubkey::Pubkey::from_str(wallet)
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;
        let us_config = self.provider.universalsettle().ok_or_else(|| {
            X402SchemeFacilitatorError::OnchainFailure("UniversalSettle not enabled".to_string())
        })?;
        let fee_bps = us_config.fee_bps.unwrap_or(0);

        // MATHEMATICAL DISCOVERY: Calculate PDAs without spending SOL on-chain.
        let (vault_pda, _) = self.provider.get_vault_pda(&seller);
        let (sol_storage_pda, _) = self.provider.get_sol_storage_pda(vault_pda);
        let (config_pda, _) = self.provider.get_config_pda(&us_config.program_id);

        let mut is_sovereign = false;
        let mut provisioning_status = None;
        let mut current_fee_bps = fee_bps;

        // Fetch on-chain state to determine sovereign status and recovery progress
        if let Ok(vault_acc) = self.provider.rpc_client().get_account(&vault_pda).await {
            let data = &vault_acc.data;
            if data.len() >= 8 + std::mem::size_of::<SplitVault>() {
                let vault: &SplitVault =
                    bytemuck::from_bytes(&data[8..8 + std::mem::size_of::<SplitVault>()]);
                is_sovereign = vault.is_sovereign == 1;

                if !is_sovereign && vault.is_provisioned == 0 {
                    // Fetch global config for recovery targets
                    if let Ok(config_acc) =
                        self.provider.rpc_client().get_account(&config_pda).await
                    {
                        let c_data = &config_acc.data;
                        if c_data.len() >= 8 + std::mem::size_of::<USConfig>() {
                            let config: &USConfig = bytemuck::from_bytes(
                                &c_data[8..8 + std::mem::size_of::<USConfig>()],
                            );
                            let sol_recovered = u64::from_le_bytes(vault.sol_recovered);
                            let sol_target = u64::from_le_bytes(config.provisioning_fee_sol);

                            let spl_recovered = u64::from_le_bytes(vault.spl_recovered);
                            let spl_target = u64::from_le_bytes(config.provisioning_fee_spl);

                            current_fee_bps = u16::from_le_bytes(config.fee_bps);

                            if sol_recovered > 0 || (spl_recovered == 0) {
                                provisioning_status =
                                    Some(crate::facilitator::ProvisioningStatus {
                                        asset: "SOL".to_string(),
                                        recovered: sol_recovered.to_string(),
                                        total: sol_target.to_string(),
                                    });
                            } else if spl_recovered > 0 {
                                provisioning_status =
                                    Some(crate::facilitator::ProvisioningStatus {
                                        asset: "USDC".to_string(),
                                        recovered: spl_recovered.to_string(),
                                        total: spl_target.to_string(),
                                    });
                            }
                        }
                    }
                } else if is_sovereign {
                    if let Ok(config_acc) =
                        self.provider.rpc_client().get_account(&config_pda).await
                    {
                        let c_data = &config_acc.data;
                        if c_data.len() >= 8 + std::mem::size_of::<USConfig>() {
                            let config: &USConfig = bytemuck::from_bytes(
                                &c_data[8..8 + std::mem::size_of::<USConfig>()],
                            );
                            current_fee_bps = u16::from_le_bytes(config.discounted_fee_bps);
                        }
                    }
                }
            }
        }

        Ok(crate::facilitator::SchemeOnboardInfo {
            label: "SplitVault (Provider State)".to_string(),
            role: "Resource Provider (Seller)".to_string(),
            vault_pda: vault_pda.to_string(),
            sol_storage_pda: sol_storage_pda.to_string(),
            token_pda: None,
            fee_bps: current_fee_bps.into(),
            status: if is_sovereign {
                "Sovereign".to_string()
            } else {
                "Active".to_string()
            },
            is_sovereign,
            provisioning_status,
        })
    }

    async fn build_onboard_tx(
        &self,
        wallet: &str,
    ) -> Result<proto::v2::BuildPaymentTxResponse, X402SchemeFacilitatorError> {
        let seller = solana_pubkey::Pubkey::from_str(wallet)
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;
        let us_config = self.provider.universalsettle().ok_or_else(|| {
            X402SchemeFacilitatorError::OnchainFailure("UniversalSettle not enabled".to_string())
        })?;

        let ix = crate::chain::solana_universalsettle::build_create_vault_instruction(
            us_config.program_id,
            seller,
            seller,
        );

        let blockhash = self
            .provider
            .rpc_client()
            .get_latest_blockhash()
            .await
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;

        // Manual construction of VersionedTransaction (unsigned shell)
        let message = solana_message::v0::Message::try_compile(&seller, &[ix], &[], blockhash)
            .map_err(|e| X402SchemeFacilitatorError::OnchainFailure(e.to_string()))?;

        let tx = solana_transaction::versioned::VersionedTransaction {
            signatures: vec![solana_signature::Signature::default()],
            message: solana_message::VersionedMessage::V0(message),
        };

        let tx_b64 = STANDARD.encode(bincode::serialize(&tx).unwrap());

        Ok(proto::v2::BuildPaymentTxResponse {
            x402_version: 2,
            transaction: tx_b64,
            recent_blockhash: blockhash.to_string(),
            fee_payer: seller.to_string(),
            payer: seller.to_string(),
            payment_uid: None,
            verify_body_template: serde_json::Value::Null,
            notes: vec!["Sovereign onboarding transaction created. Sign and send to receive the 95 bps institutional discount.".to_string()],
        })
    }

    async fn upgrade(
        &self,
        request: &proto::PaymentRequired,
    ) -> Result<proto::PaymentRequired, X402SchemeFacilitatorError> {
        // Implementation remains same ...
        Ok(proto::PaymentRequired::V2(
            types::v2_upgrade(request, &self.provider)
                .map_err(|e| X402SchemeFacilitatorError::InvalidPayload(e.to_string()))?,
        ))
    }
}

impl V2SolanaExactFacilitator {
    /// Protect against "Toxic Assets" (worthless SpamTokens).
    async fn validate_mint(
        &self,
        mint: &solana_pubkey::Pubkey,
    ) -> Result<(), X402SchemeFacilitatorError> {
        use crate::parameters::resolve_allowed_payment_mints;

        let allowed = resolve_allowed_payment_mints(self.db.as_ref()).await;
        if allowed.is_empty() {
            // If the whitelist is not set in DB or Env, we permit all assets (permissive testnet mode).
            return Ok(());
        }

        let mint_str = mint.to_string();
        if allowed.contains(&mint_str) {
            return Ok(());
        }

        // Special case: Native SOL.
        // We ensure a human-friendly error if SOL is requested but not in the whitelist.
        tracing::error!(mint = %mint_str, "Unauthorized mint attempted for settlement");
        Err(X402SchemeFacilitatorError::InvalidPayload(format!(
            "Mint {} is not supported for payment by this facilitator. Approved assets: {}.",
            mint_str,
            allowed.join(", ")
        )))
    }
}

/// Verify a v2 transfer request.
pub async fn verify_transfer(
    provider: &SolanaChainProvider,
    request: &types::VerifyRequest,
) -> Result<VerifyTransferResult, PaymentVerificationError> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    let accepted = &payload.accepted;
    if accepted != requirements {
        return Err(PaymentVerificationError::AcceptedRequirementsMismatch);
    }

    let chain_id = provider.chain_id();
    let payload_chain_id = &accepted.network;
    if payload_chain_id != &chain_id {
        return Err(PaymentVerificationError::UnsupportedChain);
    }
    let transaction_b64_string = payload.payload.transaction.clone();
    let transfer_requirement = TransferRequirement {
        pay_to: &requirements.pay_to,
        asset: &requirements.asset,
        amount: requirements.amount.inner(),
        merchant_wallet: requirements
            .extra
            .as_ref()
            .and_then(|e| e.merchant_wallet.as_ref()), // IDENTITY: pass to resolver
        collection_beneficiary: requirements
            .extra
            .as_ref()
            .and_then(|e| e.beneficiary.as_ref()), // COLLECTION: priority destination
    };
    shared_verify_transaction(provider, transaction_b64_string, &transfer_requirement).await
}
