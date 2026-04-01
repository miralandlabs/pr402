//! x402 protocol types (v2 only).

pub mod util;
pub mod v2;

use crate::chain::ChainId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

pub type SettleRequest = VerifyRequest;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedPaymentKind {
    pub x402_version: u8,
    pub scheme: String,
    pub network: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedResponse {
    pub kinds: Vec<SupportedPaymentKind>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub signers: HashMap<ChainId, Vec<String>>,
}

/// Wrapper for a payment payload and requirements sent by the client to a facilitator
/// to be verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyRequest(serde_json::Value);

impl VerifyRequest {
    pub fn into_json(self) -> serde_json::Value {
        self.0
    }

    /// Prefer `` / `X-Correlation-ID` from the HTTP request, else optional body field `correlationId`.
    ///
    /// When absent and Postgres is enabled (`DATABASE_URL`), the serverless facilitator may mint an id on
    /// **successful** `/verify`, return it as `correlationId` in the JSON body and `` header,
    /// and expect the same value on `/settle` to merge into one `payment_attempts` row.
    pub fn correlation_id_for_persistence<'a>(
        &'a self,
        http_correlation_header: Option<&'a str>,
    ) -> Option<String> {
        let from_header = http_correlation_header.and_then(|h| {
            let t = h.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        });
        if from_header.is_some() {
            return from_header;
        }
        self.0
            .get("correlationId")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Payee / resource provider wallet from `paymentRequirements.payTo` (Solana base58 address).
    pub fn payee_wallet(&self) -> Option<String> {
        self.0
            .get("paymentRequirements")
            .and_then(|r| r.get("payTo"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    /// Maps `paymentRequirements.asset` (fallback: `paymentPayload.accepted.asset`) to
    /// `resource_providers.settlement_mode` / `spl_mint` per `migrations/init.sql` (`native_sol` | `spl`).
    pub fn resource_provider_settlement(&self) -> (String, Option<String>) {
        let asset = self
            .0
            .get("paymentRequirements")
            .and_then(|r| r.get("asset"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                self.0
                    .get("paymentPayload")
                    .and_then(|p| p.get("accepted"))
                    .and_then(|a| a.get("asset"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("")
            .trim();
        const NATIVE: &str = "11111111111111111111111111111111";
        const WSOL: &str = "So11111111111111111111111111111111111111112";
        if asset.is_empty() || asset == NATIVE || asset == WSOL {
            ("native_sol".to_owned(), None)
        } else {
            ("spl".to_owned(), Some(asset.to_owned()))
        }
    }

    /// Extract common x402 V2 metadata for database persistence.
    pub fn v2_metadata(
        &self,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let req = self.0.get("paymentRequirements");
        let pay_to = req
            .and_then(|r| r.get("payTo"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let scheme = req
            .and_then(|r| r.get("scheme"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let amount = req
            .and_then(|r| r.get("amount"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let asset = req
            .and_then(|r| r.get("asset"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        (pay_to, scheme, amount, asset)
    }

    pub fn scheme_handler_slug(&self) -> Option<SchemeHandlerSlug> {
        let x402_version = self.0.get("x402Version")?.as_u64()?;
        // Only support v2
        if x402_version != 2 {
            return None;
        }
        let chain_id_string = self
            .0
            .get("paymentPayload")?
            .get("accepted")?
            .get("network")?
            .as_str()?;
        let chain_id = ChainId::from_str(chain_id_string).ok()?;
        let scheme = self
            .0
            .get("paymentPayload")?
            .get("accepted")?
            .get("scheme")?
            .as_str()?;
        Some(SchemeHandlerSlug::new(chain_id, 2, scheme.into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResponse(serde_json::Value);

impl VerifyResponse {
    pub fn new(value: serde_json::Value) -> Self {
        Self(value)
    }

    pub fn into_json(self) -> serde_json::Value {
        self.0
    }
}

/// Wrapper for a payment payload and requirements sent by the client to a facilitator
/// to be verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettleResponse(serde_json::Value);

impl SettleResponse {
    pub fn new(value: serde_json::Value) -> Self {
        Self(value)
    }

    pub fn into_json(self) -> serde_json::Value {
        self.0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PaymentVerificationError {
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
    #[error("Payment amount is invalid with respect to the payment requirements")]
    InvalidPaymentAmount,
    #[error("Payment authorization is not yet valid")]
    Early,
    #[error("Payment authorization is expired")]
    Expired,
    #[error("Payment chain id is invalid with respect to the payment requirements")]
    ChainIdMismatch,
    #[error("Payment recipient is invalid with respect to the payment requirements")]
    RecipientMismatch,
    #[error("Payment asset is invalid with respect to the payment requirements")]
    AssetMismatch,
    #[error("Onchain balance is not enough to cover the payment amount")]
    InsufficientFunds,
    #[error("{0}")]
    InvalidSignature(String),
    #[error("{0}")]
    TransactionSimulation(String),
    #[error("Unsupported chain")]
    UnsupportedChain,
    #[error("Unsupported scheme")]
    UnsupportedScheme,
    #[error("Accepted does not match payment requirements")]
    AcceptedRequirementsMismatch,
}

impl From<serde_json::Error> for PaymentVerificationError {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidFormat(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorReason {
    InvalidFormat,
    InvalidPaymentAmount,
    InvalidPaymentEarly,
    InvalidPaymentExpired,
    ChainIdMismatch,
    RecipientMismatch,
    AssetMismatch,
    AcceptedRequirementsMismatch,
    InvalidSignature,
    TransactionSimulation,
    InsufficientFunds,
    UnsupportedChain,
    UnsupportedScheme,
    UnexpectedError,
}

pub trait AsPaymentProblem {
    fn as_payment_problem(&self) -> PaymentProblem;
}

pub struct PaymentProblem {
    reason: ErrorReason,
    details: String,
}

impl PaymentProblem {
    pub fn new(reason: ErrorReason, details: String) -> Self {
        Self { reason, details }
    }
    pub fn reason(&self) -> ErrorReason {
        self.reason
    }
    pub fn details(&self) -> &str {
        &self.details
    }
}

impl AsPaymentProblem for PaymentVerificationError {
    fn as_payment_problem(&self) -> PaymentProblem {
        let error_reason = match self {
            PaymentVerificationError::InvalidFormat(_) => ErrorReason::InvalidFormat,
            PaymentVerificationError::InvalidPaymentAmount => ErrorReason::InvalidPaymentAmount,
            PaymentVerificationError::InsufficientFunds => ErrorReason::InsufficientFunds,
            PaymentVerificationError::Early => ErrorReason::InvalidPaymentEarly,
            PaymentVerificationError::Expired => ErrorReason::InvalidPaymentExpired,
            PaymentVerificationError::ChainIdMismatch => ErrorReason::ChainIdMismatch,
            PaymentVerificationError::RecipientMismatch => ErrorReason::RecipientMismatch,
            PaymentVerificationError::AssetMismatch => ErrorReason::AssetMismatch,
            PaymentVerificationError::InvalidSignature(_) => ErrorReason::InvalidSignature,
            PaymentVerificationError::TransactionSimulation(_) => {
                ErrorReason::TransactionSimulation
            }
            PaymentVerificationError::UnsupportedChain => ErrorReason::UnsupportedChain,
            PaymentVerificationError::UnsupportedScheme => ErrorReason::UnsupportedScheme,
            PaymentVerificationError::AcceptedRequirementsMismatch => {
                ErrorReason::AcceptedRequirementsMismatch
            }
        };
        PaymentProblem::new(error_reason, self.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PaymentRequired {
    V2(v2::PaymentRequired),
}

/// Scheme handler slug for routing requests to the correct handler.
#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct SchemeHandlerSlug {
    pub chain_id: ChainId,
    pub x402_version: u8,
    pub name: String,
}

impl SchemeHandlerSlug {
    pub fn new(chain_id: ChainId, x402_version: u8, name: String) -> Self {
        Self {
            chain_id,
            x402_version,
            name,
        }
    }
}

impl fmt::Display for SchemeHandlerSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:v{}:{}",
            self.chain_id.namespace, self.chain_id.reference, self.x402_version, self.name
        )
    }
}
