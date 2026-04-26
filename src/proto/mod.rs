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
#[serde(rename_all = "camelCase")]
pub struct VerifyRequest {
    pub x402_version: u8,
    pub payment_payload: serde_json::Value,
    pub payment_requirements: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Maps x402 payment `asset` mint string to `resource_providers` rail (same rules as
/// [`VerifyRequest::resource_provider_settlement`]).
pub fn settlement_rail_from_x402_asset(asset: &str) -> (String, Option<String>) {
    let asset = asset.trim();
    const NATIVE: &str = "11111111111111111111111111111111";
    const WSOL: &str = "So11111111111111111111111111111111111111112";
    if asset.is_empty() || asset == NATIVE || asset == WSOL {
        ("native_sol".to_owned(), None)
    } else {
        ("spl".to_owned(), Some(asset.to_owned()))
    }
}

impl VerifyRequest {
    pub fn into_json(self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
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
        self.correlation_id.clone().filter(|s| !s.trim().is_empty())
    }

    /// Payee / resource provider wallet from `paymentRequirements.payTo` (Solana base58 address).
    pub fn payee_wallet(&self) -> Option<String> {
        self.payment_requirements
            .get("payTo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
    }

    /// Maps `paymentRequirements.asset` (fallback: `paymentPayload.accepted.asset`) to
    /// `resource_providers.settlement_mode` / `spl_mint` per `migrations/init.sql` (`native_sol` | `spl`).
    pub fn resource_provider_settlement(&self) -> (String, Option<String>) {
        let asset = self
            .payment_requirements
            .get("asset")
            .and_then(|v| v.as_str())
            .or_else(|| {
                self.payment_payload
                    .get("accepted")
                    .and_then(|a| a.get("asset"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("")
            .trim();
        settlement_rail_from_x402_asset(asset)
    }

    /// Seller pubkey (base58) for facilitator DB policy: `extra.merchantWallet` / `beneficiary` on
    /// `paymentRequirements` or nested `accepted.extra` / payload `accepted.extra`.
    pub fn resource_provider_merchant_wallet(&self) -> Option<String> {
        fn pick(extra: &serde_json::Value) -> Option<String> {
            extra
                .get("merchantWallet")
                .or_else(|| extra.get("beneficiary"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        }
        let req = &self.payment_requirements;
        if let Some(e) = req.get("extra") {
            if let Some(m) = pick(e) {
                return Some(m);
            }
        }
        if let Some(a) = req.get("accepted") {
            if let Some(ex) = a.get("extra") {
                if let Some(m) = pick(ex) {
                    return Some(m);
                }
            }
        }
        if let Some(a) = self.payment_payload.get("accepted") {
            if let Some(ex) = a.get("extra") {
                if let Some(m) = pick(ex) {
                    return Some(m);
                }
            }
        }
        None
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
        let req = &self.payment_requirements;
        let pay_to = req
            .get("payTo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let scheme = req
            .get("scheme")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let amount = req
            .get("amount")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let asset = req
            .get("asset")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        (pay_to, scheme, amount, asset)
    }

    pub fn scheme_handler_slug(&self) -> Option<SchemeHandlerSlug> {
        if self.x402_version != 2 {
            return None;
        }
        let chain_id_string = self
            .payment_payload
            .get("accepted")?
            .get("network")?
            .as_str()?;
        let chain_id = ChainId::from_str(chain_id_string).ok()?;
        let scheme = self
            .payment_payload
            .get("accepted")?
            .get("scheme")?
            .as_str()?;
        Some(SchemeHandlerSlug::new(chain_id, 2, scheme.into()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResponse {
    pub is_valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
}

impl VerifyResponse {
    pub fn valid(payer: String) -> Self {
        Self {
            is_valid: true,
            invalid_reason: None,
            payer: Some(payer),
        }
    }

    pub fn invalid(reason: String) -> Self {
        Self {
            is_valid: false,
            invalid_reason: Some(reason),
            payer: None,
        }
    }

    pub fn into_json(self) -> serde_json::Value {
        serde_json::to_value(self).unwrap()
    }
}

/// Response from settlement execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettleResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
}

impl SettleResponse {
    pub fn success(payer: String, transaction: String, network: String) -> Self {
        Self {
            success: true,
            error_reason: None,
            payer: Some(payer),
            transaction: Some(transaction),
            network: Some(network),
        }
    }

    pub fn error(reason: String, network: String) -> Self {
        Self {
            success: false,
            error_reason: Some(reason),
            payer: None,
            transaction: None,
            network: Some(network),
        }
    }

    pub fn into_json(self) -> serde_json::Value {
        serde_json::to_value(self).unwrap()
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
