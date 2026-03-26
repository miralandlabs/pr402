//! Types for v2:solana:exact scheme.

use std::fmt;

use crate::chain::solana::Address;
use crate::proto::util::U64String;
use crate::proto::v2;
use serde::{Deserialize, Serialize};

/// Exact scheme identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactScheme;

impl AsRef<str> for ExactScheme {
    fn as_ref(&self) -> &str {
        "exact"
    }
}

impl fmt::Display for ExactScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("exact")
    }
}

/// Solana exact payload (base64-encoded transaction).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactSolanaPayload {
    pub transaction: String,
}

/// Supported payment kind extra information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SupportedPaymentKindExtra {
    pub fee_payer: Address,
}

/// Verify request for v2:solana:exact.
pub type VerifyRequest = v2::VerifyRequest<PaymentPayload, PaymentRequirements>;

/// Settle request (same as verify).
pub type SettleRequest = VerifyRequest;

/// Payment payload for v2:solana:exact.
pub type PaymentPayload = v2::PaymentPayload<PaymentRequirements, ExactSolanaPayload>;

/// Payment requirements for v2:solana:exact.
pub type PaymentRequirements =
    v2::PaymentRequirements<ExactScheme, U64String, Address, SupportedPaymentKindExtra>;
