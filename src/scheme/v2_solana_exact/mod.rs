//! v2:solana:exact payment scheme implementation.

pub mod types;

use std::collections::HashMap;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

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
        _db: Option<crate::db::Pr402Db>,
    ) -> Result<Box<dyn X402SchemeFacilitator>, Box<dyn Error>> {
        Ok(Box::new(V2SolanaExactFacilitator {
            provider: provider.solana,
        }))
    }
}

pub struct V2SolanaExactFacilitator {
    provider: Arc<SolanaChainProvider>,
}

#[async_trait::async_trait]
impl X402SchemeFacilitator for V2SolanaExactFacilitator {
    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, X402SchemeFacilitatorError> {
        let request = types::VerifyRequest::from_proto(request.clone())?;
        let verification = verify_transfer(&self.provider, &request).await?;
        Ok(proto::v2::VerifyResponse::valid(verification.payer.to_string()).into())
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeFacilitatorError> {
        let request = types::SettleRequest::from_proto(request.clone())?;
        let verification = verify_transfer(&self.provider, &request).await?;

        // Just-in-Time Provisioning: Ensure vault is created only when a real settlement is requested.
        // This prevents spam-creation of accounts as settlement requires valid, signed payloads.
        if let Some(us_config) = self.provider.universalsettle() {
            let seller = *verification.beneficiary.pubkey();
            let fee_dest = us_config.fee_destination.ok_or_else(|| {
                X402SchemeFacilitatorError::OnchainFailure(
                    "UniversalSettle fee destination not configured".to_string(),
                )
            })?;
            let fee_bps = us_config.fee_bps.unwrap_or(100);
            let asset = *request.payment_requirements.asset.pubkey();
            self.provider
                .ensure_vault_setup(&seller, &fee_dest, fee_bps, Some(asset))
                .await?;
        }

        let payer = verification.payer.to_string();
        let tx_sig = settle_transaction(&self.provider, verification).await?;
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
            let fee_payer = self.provider.fee_payer();
            let extra = if let Some(us_config) = self.provider.universalsettle() {
                let (config_address, _) = self.provider.get_config_pda(&us_config.program_id);
                let fee_bps = us_config.fee_bps.unwrap_or(0);

                Some(
                    serde_json::to_value(SupportedPaymentKindExtra {
                        fee_payer: fee_payer.into(),
                        program_id: us_config.program_id.into(),
                        config_address: config_address.into(),
                        fee_bps: fee_bps.into(),
                    })
                    .unwrap(),
                )
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

        Ok(crate::facilitator::SchemeOnboardInfo {
            label: "SplitVault (Provider State)".to_string(),
            role: "Resource Provider (Seller)".to_string(),
            vault_pda: vault_pda.to_string(),
            sol_storage_pda: sol_storage_pda.to_string(),
            token_pda: None, // Only provisioned for specific SPL mints during settlement
            fee_bps: fee_bps.into(),
            status: "Discovery".to_string(),
        })
    }

    async fn upgrade(
        &self,
        request: &proto::PaymentRequired,
    ) -> Result<proto::PaymentRequired, X402SchemeFacilitatorError> {
        let mut pr = match request {
            proto::PaymentRequired::V2(v2) => v2.clone(),
        };

        for (i, accept) in pr.accepts.iter_mut().enumerate() {
            let scheme = accept.get("scheme").and_then(|s| s.as_str());
            let network = accept.get("network").and_then(|n| n.as_str());

            if scheme == Some(ExactScheme.as_ref())
                && network == Some(&self.provider.chain_id().to_string())
            {
                let pay_to = accept.get("payTo").and_then(|p| p.as_str()).unwrap_or("");
                if let Ok(merchant_wallet) = solana_pubkey::Pubkey::from_str(pay_to) {
                    // SE-CRIT-11: Institutional Elevation.
                    // If the merchant provided a raw wallet, we elevate it to their SplitVault PDA.
                    let (vault_pda, _) = self.provider.get_vault_pda(&merchant_wallet);

                    if let Some(obj) = accept.as_object_mut() {
                        obj.insert(
                            "payTo".to_string(),
                            serde_json::json!(vault_pda.to_string()),
                        );

                        // Inject required institutional metadata for agentic signers
                        if let Some(us_config) = self.provider.universalsettle() {
                            let (config_address, _) =
                                self.provider.get_config_pda(&us_config.program_id);
                            let extra = SupportedPaymentKindExtra {
                                fee_payer: self.provider.fee_payer().into(),
                                program_id: us_config.program_id.into(),
                                config_address: config_address.into(),
                                fee_bps: us_config.fee_bps.unwrap_or(0).into(),
                            };
                            obj.insert("extra".to_string(), serde_json::to_value(extra).unwrap());
                        }

                        tracing::info!(
                            index = i,
                            merchant = %merchant_wallet,
                            vault = %vault_pda,
                            "Upgraded Lite challenge to institutional SplitVault"
                        );
                    }
                }
            }
        }

        Ok(proto::PaymentRequired::V2(pr))
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
    };
    shared_verify_transaction(provider, transaction_b64_string, &transfer_requirement).await
}
