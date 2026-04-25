//! Types for v2:solana:escrow scheme.

use std::fmt;

use crate::chain::solana::Address;
use crate::proto::util::{U16String, U64String};
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
    pub fee_bps: U16String,
    pub oracle_fee_bps: U16String,
    pub ttl_seconds: U64String,
    /// Who pays Solana **network** fees on the default `build-sla-escrow-payment-tx` shell:
    /// `"facilitator"` (recommended; aligns with `exact`) or `"buyer"` (legacy / CLI-shaped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sla_fund_tx_network_fee_payer: Option<String>,
    #[serde(default)]
    pub merchant_wallet: Option<Address>, // IDENTITY: Original wallet for re-derivation
    #[serde(default)]
    pub beneficiary: Option<Address>, // COLLECTION: Final payout destination (Priority)
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
