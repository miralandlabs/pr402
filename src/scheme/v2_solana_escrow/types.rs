//! Types for v2:solana:escrow scheme.

use std::fmt;

use crate::chain::solana::Address;
use crate::proto::util::U64String;
use crate::proto::v2;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// SLAEscrow scheme identifier.
///
/// Wire format is the string `"sla-escrow"` (x402 `PaymentRequirements.scheme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SLAEscrowScheme;

impl Serialize for SLAEscrowScheme {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(SLAEscrowScheme.as_ref())
    }
}

impl<'de> Deserialize<'de> for SLAEscrowScheme {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        if s == SLAEscrowScheme.as_ref() {
            Ok(SLAEscrowScheme)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected scheme {:?}, got {:?}",
                SLAEscrowScheme.as_ref(),
                s
            )))
        }
    }
}

impl AsRef<str> for SLAEscrowScheme {
    fn as_ref(&self) -> &str {
        "sla-escrow"
    }
}

impl fmt::Display for SLAEscrowScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sla-escrow")
    }
}

/// Solana escrow payload (base64-encoded transaction).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SLAEscrowSolanaPayload {
    pub transaction: String,
}

/// Supported payment kind extra information for escrow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SLAEscrowPaymentRequirementsExtra {
    pub fee_payer: Address,
    pub oracle_authorities: Vec<Address>,
    pub escrow_program_id: Address,
    pub bank_address: Address,
    pub config_address: Address,
    pub fee_bps: u16,
    pub ttl_seconds: u64,
}

/// Verify request for v2:solana:escrow.
pub type VerifyRequest = v2::VerifyRequest<PaymentPayload, PaymentRequirements>;

/// Settle request (same as verify).
pub type SettleRequest = VerifyRequest;

/// Payment payload for v2:solana:escrow.
pub type PaymentPayload = v2::PaymentPayload<PaymentRequirements, SLAEscrowSolanaPayload>;

/// Payment requirements for v2:solana:escrow.
pub type PaymentRequirements =
    v2::PaymentRequirements<SLAEscrowScheme, U64String, Address, SLAEscrowPaymentRequirementsExtra>;
