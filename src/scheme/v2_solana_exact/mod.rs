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
            let extra = Some(
                serde_json::to_value(SupportedPaymentKindExtra {
                    fee_payer: fee_payer.into(),
                })
                .unwrap(),
            );
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
        let _fee_dest = us_config.fee_destination.ok_or_else(|| {
            X402SchemeFacilitatorError::OnchainFailure(
                "UniversalSettle fee destination not configured".to_string(),
            )
        })?;
        let fee_bps = us_config.fee_bps.unwrap_or(0);

        // MATHEMATICAL DISCOVERY: Calculate PDAs without spending SOL on-chain.
        let (vault_pda, _) = self.provider.get_vault_pda(&seller);
        let (sol_storage_pda, _) = self.provider.get_sol_storage_pda(vault_pda);

        Ok(crate::facilitator::SchemeOnboardInfo {
            vault_pda: vault_pda.to_string(),
            sol_storage_pda: sol_storage_pda.to_string(),
            fee_bps,
            status: "Discovery".to_string(), // status indicates it is derived but not necessarily provisioned
        })
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
