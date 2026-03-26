//! x402 payment scheme implementations (v2:solana:exact only).

pub mod v2_solana_escrow;
pub mod v2_solana_exact;

use crate::chain::ChainProvider;
use crate::facilitator::SchemeOnboardInfo;
use crate::proto;
use std::fmt::Debug;

/// Trait for x402 scheme facilitators.
#[async_trait::async_trait]
pub trait X402SchemeFacilitator: Send + Sync {
    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, X402SchemeFacilitatorError>;
    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeFacilitatorError>;
    async fn supported(&self) -> Result<proto::SupportedResponse, X402SchemeFacilitatorError>;
    async fn onboard(&self, wallet: &str) -> Result<SchemeOnboardInfo, X402SchemeFacilitatorError>;
}

/// Scheme identifier trait.
pub trait X402SchemeId {
    fn x402_version(&self) -> u8 {
        2
    }
    fn namespace(&self) -> &str;
    fn scheme(&self) -> &str;
}

/// Scheme facilitator builder trait.
pub trait X402SchemeFacilitatorBuilder {
    fn build(
        &self,
        provider: ChainProvider,
        config: Option<serde_json::Value>,
    ) -> Result<Box<dyn X402SchemeFacilitator>, Box<dyn std::error::Error>>;
}

#[derive(Debug, thiserror::Error)]
pub enum X402SchemeFacilitatorError {
    #[error(transparent)]
    PaymentVerification(#[from] proto::PaymentVerificationError),
    #[error(transparent)]
    SolanaChain(#[from] crate::chain::solana::SolanaChainProviderError),
    #[error("Onchain error: {0}")]
    OnchainFailure(String),
}

impl proto::AsPaymentProblem for X402SchemeFacilitatorError {
    fn as_payment_problem(&self) -> proto::PaymentProblem {
        match self {
            X402SchemeFacilitatorError::PaymentVerification(e) => e.as_payment_problem(),
            X402SchemeFacilitatorError::SolanaChain(e) => {
                proto::PaymentProblem::new(proto::ErrorReason::TransactionSimulation, e.to_string())
            }
            X402SchemeFacilitatorError::OnchainFailure(e) => {
                proto::PaymentProblem::new(proto::ErrorReason::UnexpectedError, e.to_string())
            }
        }
    }
}
