//! Core facilitator trait and implementation.

use crate::chain::ChainProvider;
use crate::proto;
use crate::scheme::v2_solana_escrow::V2SolanaSLAEscrow;
use crate::scheme::v2_solana_exact::V2SolanaExact;
use crate::scheme::{
    X402SchemeFacilitator, X402SchemeFacilitatorBuilder, X402SchemeFacilitatorError, X402SchemeId,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

/// Trait defining the asynchronous interface for x402 payment facilitators.
#[async_trait]
pub trait Facilitator: Send + Sync {
    /// The error type returned by this facilitator.
    type Error: std::fmt::Debug + std::fmt::Display + Send + Sync;

    /// Verifies a proposed x402 payment payload against requirements.
    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, Self::Error>;

    /// Executes an on-chain x402 settlement for a valid request.
    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, Self::Error>;

    /// Lists supported payment schemes and networks.
    async fn supported(&self) -> Result<proto::SupportedResponse, Self::Error>;

    /// Onboards a resource owner's wallet by ensuring vaults are provisioned.
    async fn onboard(&self, wallet: &str) -> Result<OnboardResponse, Self::Error>;

    /// Builds an unsigned transaction for proactive onboarding to become Sovereign.
    async fn build_onboard_tx(
        &self,
        wallet: &str,
    ) -> Result<proto::v2::BuildPaymentTxResponse, Self::Error>;

    /// Upgrades a Lite 402 challenge into a full Institutional PaymentRequired response.
    async fn upgrade(
        &self,
        request: &proto::PaymentRequired,
    ) -> Result<proto::PaymentRequired, Self::Error>;
}

/// Information returned after onboarding a wallet for a specific scheme.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SchemeOnboardInfo {
    pub label: String,
    pub role: String,
    pub vault_pda: String,
    pub sol_storage_pda: String,
    pub token_pda: Option<String>,
    pub fee_bps: crate::proto::util::U16String,
    pub status: String,
    pub is_sovereign: bool,
    pub provisioning_status: Option<ProvisioningStatus>,
}

/// Recovery progress for JIT provisioned vaults.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProvisioningStatus {
    pub asset: String,     // "SOL" or "USDC" (mint symbol)
    pub recovered: String, // Raw units as string
    pub total: String,     // Raw units as string
}

/// Complete onboarding response for a wallet across all supported schemes.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OnboardResponse {
    pub wallet: String,
    pub facilitator: String,
    pub schemes: HashMap<String, SchemeOnboardInfo>,
}

/// Local facilitator implementation supporting multiple Solana schemes.
pub struct FacilitatorLocal {
    scheme_handlers: HashMap<String, Arc<dyn X402SchemeFacilitator>>,
}

impl FacilitatorLocal {
    /// Create a new facilitator with the given chain provider and optional database.
    pub fn new(
        chain_provider: ChainProvider,
        db: Option<crate::db::Pr402Db>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut scheme_handlers: HashMap<String, Arc<dyn X402SchemeFacilitator>> = HashMap::new();

        // Always register exact scheme
        let exact_scheme = V2SolanaExact;
        scheme_handlers.insert(
            exact_scheme.scheme().to_string(),
            Arc::from(exact_scheme.build(chain_provider.clone(), None, db.clone())?),
        );

        // Register escrow scheme if configured
        if let Some(escrow_config) = chain_provider.solana.sla_escrow() {
            let escrow_scheme = V2SolanaSLAEscrow;
            match escrow_scheme.build(chain_provider.clone(), None, db.clone()) {
                Ok(handler) => {
                    scheme_handlers.insert(escrow_scheme.scheme().to_string(), Arc::from(handler));
                    info!(
                        "Registered escrow scheme for program: {}",
                        escrow_config.program_id
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to build escrow scheme: {}. Is ESCROW_PROGRAM_ID correct?",
                        e
                    );
                    return Err(e);
                }
            }
        }

        Ok(Self { scheme_handlers })
    }
}

#[async_trait]
impl Facilitator for FacilitatorLocal {
    type Error = FacilitatorLocalError;

    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, Self::Error> {
        // We only support v2 right now, extract scheme from payload based on generic request parsing
        // Since we only need the scheme, we can peek into the JSON.
        let val: serde_json::Value =
            serde_json::from_value(request.clone().into_json()).unwrap_or_default();
        let scheme = val
            .get("paymentRequirements")
            .and_then(|r| r.get("scheme"))
            .and_then(|s| s.as_str())
            .unwrap_or("exact");

        let handler = self.scheme_handlers.get(scheme).ok_or_else(|| {
            FacilitatorLocalError::Verification(X402SchemeFacilitatorError::OnchainFailure(
                format!("Unsupported scheme: {}", scheme),
            ))
        })?;

        handler
            .verify(request)
            .await
            .map_err(FacilitatorLocalError::Verification)
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, Self::Error> {
        let val: serde_json::Value =
            serde_json::from_value(request.clone().into_json()).unwrap_or_default();
        let scheme = val
            .get("paymentRequirements")
            .and_then(|r| r.get("scheme"))
            .and_then(|s| s.as_str())
            .unwrap_or("exact");

        let handler = self.scheme_handlers.get(scheme).ok_or_else(|| {
            FacilitatorLocalError::Settlement(X402SchemeFacilitatorError::OnchainFailure(format!(
                "Unsupported scheme: {}",
                scheme
            )))
        })?;

        handler
            .settle(request)
            .await
            .map_err(FacilitatorLocalError::Settlement)
    }

    async fn supported(&self) -> Result<proto::SupportedResponse, Self::Error> {
        let mut supported_response = proto::SupportedResponse {
            kinds: Vec::new(),
            extensions: Vec::new(),
            signers: HashMap::new(),
        };

        for handler in self.scheme_handlers.values() {
            if let Ok(response) = handler.supported().await {
                supported_response.kinds.extend(response.kinds);
                supported_response.extensions.extend(response.extensions);
                // Merge signers
                for (k, v) in response.signers {
                    supported_response.signers.entry(k).or_default().extend(v);
                }
            }
        }

        // Deduplicate signers
        for signers in supported_response.signers.values_mut() {
            signers.sort();
            signers.dedup();
        }

        Ok(supported_response)
    }

    async fn onboard(&self, wallet: &str) -> Result<OnboardResponse, Self::Error> {
        let mut response = OnboardResponse {
            wallet: wallet.to_string(),
            facilitator: "pr402".to_string(), // TODO: Get facilitator name/id
            schemes: HashMap::new(),
        };

        for (name, handler) in &self.scheme_handlers {
            if let Ok(onboard_info) = handler.onboard(wallet).await {
                response.schemes.insert(name.clone(), onboard_info);
            }
        }

        Ok(response)
    }

    async fn build_onboard_tx(
        &self,
        wallet: &str,
    ) -> Result<proto::v2::BuildPaymentTxResponse, Self::Error> {
        // UniversalSettle Proactive Onboarding: Any scheme handler can build it since they
        // all share the same UniversalSettle infrastructure. We pick the first one.
        for handler in self.scheme_handlers.values() {
            if let Ok(tx) = handler.build_onboard_tx(wallet).await {
                return Ok(tx);
            }
        }

        Err(FacilitatorLocalError::Onboard(
            X402SchemeFacilitatorError::OnchainFailure(
                "No scheme handler could build onboarding transaction".to_string(),
            ),
        ))
    }

    async fn upgrade(
        &self,
        request: &proto::PaymentRequired,
    ) -> Result<proto::PaymentRequired, Self::Error> {
        let mut upgraded = request.clone();
        for handler in self.scheme_handlers.values() {
            upgraded = handler
                .upgrade(&upgraded)
                .await
                .map_err(FacilitatorLocalError::Upgrade)?;
        }
        Ok(upgraded)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FacilitatorLocalError {
    #[error(transparent)]
    Verification(X402SchemeFacilitatorError),
    #[error(transparent)]
    Settlement(X402SchemeFacilitatorError),
    #[error(transparent)]
    Upgrade(X402SchemeFacilitatorError),
    #[error(transparent)]
    Onboard(X402SchemeFacilitatorError),
}
