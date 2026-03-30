//! Types for v2:solana:exact scheme.

use std::fmt;

use crate::chain::solana::Address;
use crate::proto::util::{U16String, U64String};
use crate::proto::v2;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Exact scheme identifier.
///
/// Wire format is the string `"exact"` (x402 `PaymentRequirements.scheme`), not a unit JSON value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactScheme;

impl Serialize for ExactScheme {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(ExactScheme.as_ref())
    }
}

impl<'de> Deserialize<'de> for ExactScheme {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        if s == ExactScheme.as_ref() {
            Ok(ExactScheme)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected scheme {:?}, got {:?}",
                ExactScheme.as_ref(),
                s
            )))
        }
    }
}

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
    pub program_id: Address,
    pub config_address: Address,
    pub fee_bps: U16String,
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
